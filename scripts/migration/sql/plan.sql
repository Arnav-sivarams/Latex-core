\set ON_ERROR_STOP on
SET client_min_messages = warning;
SET search_path = pg_catalog, public;

DO $validation$
DECLARE
  required_table text;
BEGIN
  IF to_regclass('public._sqlx_migrations') IS NULL THEN
    RAISE EXCEPTION 'restore validation failed: migration ledger is missing';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM public._sqlx_migrations WHERE success) THEN
    RAISE EXCEPTION 'restore validation failed: migration ledger has no successful entry';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'latex_core') THEN
    RAISE EXCEPTION 'restore validation failed: latex_core schema is missing';
  END IF;
  FOREACH required_table IN ARRAY ARRAY[
    'users', 'user_credentials', 'projects', 'teams', 'team_members',
    'team_projects', 'team_project_members', 'member_drafts',
    'member_change_operations', 'research_groups', 'research_group_members',
    'templates', 'template_files', 'file_policies', 'audit_events',
    'compile_jobs', 'compilation_artifacts'
  ] LOOP
    IF to_regclass(format('latex_core.%I', required_table)) IS NULL THEN
      RAISE EXCEPTION 'restore validation failed: table % is missing', required_table;
    END IF;
  END LOOP;
END
$validation$;

CREATE TEMP TABLE reconciliation (
  ordinal integer PRIMARY KEY,
  metric text NOT NULL,
  expected bigint NOT NULL,
  actual bigint NOT NULL
);

INSERT INTO reconciliation (ordinal, metric, expected, actual) VALUES
  (1,  'users', 147, (SELECT count(*) FROM latex_core.users)),
  (2,  'student', 95, (SELECT count(*) FROM latex_core.user_credentials WHERE account_type = 'student')),
  (3,  'professor', 28, (SELECT count(*) FROM latex_core.user_credentials WHERE account_type = 'professor')),
  (4,  'admin', 20, (SELECT count(*) FROM latex_core.user_credentials WHERE account_type = 'admin')),
  (5,  'users_lacking_credential_account_type', 4,
       (SELECT count(*) FROM latex_core.users u LEFT JOIN latex_core.user_credentials c ON c.user_id = u.id WHERE c.user_id IS NULL)),
  (6,  'personal_projects', 73, (SELECT count(*) FROM latex_core.projects)),
  (7,  'teams', 43, (SELECT count(*) FROM latex_core.teams)),
  (8,  'team_projects', 41, (SELECT count(*) FROM latex_core.team_projects)),
  (9,  'team_memberships', 115, (SELECT count(*) FROM latex_core.team_members)),
  (10, 'project_role_memberships', 97, (SELECT count(*) FROM latex_core.team_project_members)),
  (11, 'research_groups', 1, (SELECT count(*) FROM latex_core.research_groups)),
  (12, 'templates', 5, (SELECT count(*) FROM latex_core.templates)),
  (13, 'file_policy_rules', 7, (SELECT count(*) FROM latex_core.file_policies)),
  (14, 'users_with_overlapping_role_conflicts', 25,
       (SELECT count(DISTINCT user_id) FROM latex_core.team_project_members
        WHERE writer::integer + mentor::integer + project_manager::integer > 1)),
  (15, 'overlapping_project_role_memberships', 45,
       (SELECT count(*) FROM latex_core.team_project_members
        WHERE writer::integer + mentor::integer + project_manager::integer > 1)),
  (16, 'multi_project_teams', 7,
       (SELECT count(*) FROM (SELECT team_id FROM latex_core.team_projects GROUP BY team_id HAVING count(*) > 1) grouped)),
  (17, 'users_with_unpublished_private_work', 13,
       (SELECT count(DISTINCT user_id) FROM (
          SELECT user_id FROM latex_core.member_drafts
          UNION
          SELECT user_id FROM latex_core.member_change_operations
        ) private_users)),
  (18, 'unpublished_change_sets', 13,
       (SELECT count(*) FROM (
          SELECT team_project_id, user_id FROM latex_core.member_drafts
          UNION
          SELECT team_project_id, user_id FROM latex_core.member_change_operations
        ) change_sets)),
  (19, 'draft_files', 16, (SELECT count(*) FROM latex_core.member_drafts)),
  (20, 'structural_operations', 17, (SELECT count(*) FROM latex_core.member_change_operations));

\copy (SELECT metric, expected, actual, CASE WHEN expected = actual THEN 'MATCH' ELSE 'MISMATCH' END AS status FROM reconciliation ORDER BY ordinal) TO '/output/reconciliation.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

DO $reconcile$
BEGIN
  IF EXISTS (SELECT 1 FROM reconciliation WHERE expected <> actual) THEN
    RAISE EXCEPTION 'source snapshot does not match the frozen C0 baseline';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM latex_core.member_drafts) OR
     NOT EXISTS (SELECT 1 FROM latex_core.research_groups) OR
     NOT EXISTS (SELECT 1 FROM latex_core.templates) OR
     NOT EXISTS (SELECT 1 FROM latex_core.audit_events) OR
     NOT EXISTS (SELECT 1 FROM latex_core.compile_jobs) THEN
    RAISE EXCEPTION 'restore validation failed: required prototype metadata is empty';
  END IF;
END
$reconcile$;

CREATE TEMP VIEW private_change_sets AS
SELECT team_project_id, user_id
FROM latex_core.member_drafts
UNION
SELECT team_project_id, user_id
FROM latex_core.member_change_operations;

CREATE TEMP VIEW user_signals AS
SELECT
  u.id AS user_id,
  COALESCE(c.email, '') AS email,
  (c.user_id IS NOT NULL) AS credential_present,
  c.enabled AS enabled,
  COALESCE(c.account_type, '') AS legacy_account_type,
  (SELECT count(*) FROM latex_core.projects p WHERE p.owner_user_id = u.id) AS personal_paper_count,
  (SELECT count(DISTINCT tm.team_id) FROM latex_core.team_members tm WHERE tm.user_id = u.id) AS team_count,
  (SELECT count(DISTINCT pm.team_project_id) FROM latex_core.team_project_members pm WHERE pm.user_id = u.id AND pm.writer) AS writer_project_count,
  (SELECT count(DISTINCT pm.team_project_id) FROM latex_core.team_project_members pm WHERE pm.user_id = u.id AND pm.mentor) AS mentor_project_count,
  (SELECT count(DISTINCT pm.team_project_id) FROM latex_core.team_project_members pm WHERE pm.user_id = u.id AND pm.project_manager) AS project_manager_project_count,
  (EXISTS (SELECT 1 FROM latex_core.projects p WHERE p.owner_user_id = u.id) OR
   EXISTS (SELECT 1 FROM latex_core.team_project_members pm WHERE pm.user_id = u.id AND pm.writer)) AS has_writer_signal,
  EXISTS (SELECT 1 FROM latex_core.team_project_members pm WHERE pm.user_id = u.id AND pm.mentor) AS has_mentor_signal,
  EXISTS (SELECT 1 FROM latex_core.team_project_members pm WHERE pm.user_id = u.id AND pm.project_manager) AS has_project_manager_signal,
  EXISTS (
    SELECT 1 FROM latex_core.team_project_members pm
    WHERE pm.user_id = u.id AND pm.writer::integer + pm.mentor::integer + pm.project_manager::integer > 1
  ) AS has_role_overlap,
  EXISTS (SELECT 1 FROM private_change_sets pcs WHERE pcs.user_id = u.id) AS has_unpublished_private_work
FROM latex_core.users u
LEFT JOIN latex_core.user_credentials c ON c.user_id = u.id;

\copy (SELECT user_id, email, credential_present, enabled, legacy_account_type, personal_paper_count, team_count, writer_project_count, mentor_project_count, project_manager_project_count, has_writer_signal, has_mentor_signal, has_project_manager_signal, has_role_overlap, has_unpublished_private_work FROM user_signals ORDER BY user_id) TO '/output/users.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

CREATE TEMP VIEW role_decisions AS
SELECT
  s.*,
  concat_ws(', ',
    CASE WHEN s.legacy_account_type <> '' THEN 'account:' || s.legacy_account_type END,
    CASE WHEN s.personal_paper_count > 0 THEN 'personal_papers:' || s.personal_paper_count END,
    CASE WHEN s.writer_project_count > 0 THEN 'writer_projects:' || s.writer_project_count END,
    CASE WHEN s.mentor_project_count > 0 THEN 'mentor_projects:' || s.mentor_project_count END,
    CASE WHEN s.project_manager_project_count > 0 THEN 'project_manager_projects:' || s.project_manager_project_count END,
    CASE WHEN s.has_unpublished_private_work THEN 'unpublished_private_work' END
  ) AS signals,
  CASE
    WHEN NOT s.credential_present THEN ''
    WHEN s.legacy_account_type = 'admin' THEN 'admin'
    WHEN s.has_writer_signal AND NOT s.has_mentor_signal THEN 'writer'
    WHEN s.has_mentor_signal AND NOT s.has_writer_signal AND s.personal_paper_count = 0 THEN 'mentor'
    ELSE ''
  END AS suggested_v2_role,
  CASE
    WHEN NOT s.credential_present THEN 'BLOCKED_MISSING_ACCOUNT'
    WHEN s.has_unpublished_private_work THEN 'BLOCKED_PRIVATE_WORK'
    WHEN s.legacy_account_type = 'admin' AND
         (s.has_writer_signal OR s.has_mentor_signal) THEN 'MANUAL_REQUIRED'
    WHEN s.has_writer_signal AND s.has_mentor_signal THEN 'MANUAL_REQUIRED'
    WHEN s.has_project_manager_signal THEN 'MANUAL_REQUIRED'
    WHEN s.legacy_account_type = 'admin' THEN 'AUTO_CANDIDATE'
    WHEN s.has_writer_signal AND NOT s.has_mentor_signal THEN 'AUTO_CANDIDATE'
    WHEN s.has_mentor_signal AND NOT s.has_writer_signal AND s.personal_paper_count = 0 THEN 'AUTO_CANDIDATE'
    ELSE 'MANUAL_REQUIRED'
  END AS decision_class,
  concat_ws('; ',
    CASE WHEN NOT s.credential_present THEN 'no usable credential/account-type row; identity and role must not be inferred' END,
    CASE WHEN s.has_unpublished_private_work THEN 'unpublished private work requires an explicit disposition before role conversion' END,
    CASE WHEN s.legacy_account_type = 'admin' AND s.personal_paper_count > 0 THEN 'legacy Admin owns personal papers but V2 Admin cannot own client work' END,
    CASE WHEN s.legacy_account_type = 'admin' AND (s.writer_project_count > 0 OR s.mentor_project_count > 0) THEN 'legacy Admin has Writer/Mentor client-work history' END,
    CASE WHEN s.has_writer_signal AND s.has_mentor_signal THEN 'Writer and Mentor evidence conflicts under the exclusive V2 role model' END,
    CASE WHEN s.has_project_manager_signal THEN 'Project Manager has no V2 equivalent and requires explicit resolution' END,
    CASE WHEN s.legacy_account_type = 'professor' AND NOT s.has_mentor_signal AND NOT s.has_writer_signal THEN 'legacy Professor alone is insufficient evidence for Mentor' END,
    CASE WHEN s.legacy_account_type = 'student' AND NOT s.has_writer_signal AND NOT s.has_mentor_signal AND NOT s.has_project_manager_signal THEN 'legacy Student has no role-bearing project or ownership evidence' END,
    CASE WHEN s.credential_present AND s.legacy_account_type = '' THEN 'credential has no usable account type' END,
    CASE WHEN s.credential_present AND s.legacy_account_type <> 'admin' AND NOT s.has_writer_signal AND NOT s.has_mentor_signal AND NOT s.has_project_manager_signal AND s.legacy_account_type NOT IN ('student', 'professor') THEN 'no deterministic V2 role evidence' END,
    CASE WHEN s.legacy_account_type = 'admin' AND NOT s.has_writer_signal AND NOT s.has_mentor_signal THEN 'legacy Admin has no client-work conflict' END,
    CASE WHEN s.has_writer_signal AND NOT s.has_mentor_signal AND NOT s.has_project_manager_signal AND s.legacy_account_type <> 'admin' THEN 'Writer evidence is non-conflicting' END,
    CASE WHEN s.has_mentor_signal AND NOT s.has_writer_signal AND NOT s.has_project_manager_signal AND s.personal_paper_count = 0 AND s.legacy_account_type <> 'admin' THEN 'actual Mentor assignment evidence is non-conflicting' END
  ) AS reason
FROM user_signals s;

\copy (SELECT user_id, email, legacy_account_type, signals, suggested_v2_role, decision_class, reason, NULL::text AS resolved_v2_role FROM role_decisions ORDER BY user_id) TO '/output/user-role-decisions.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

\copy (SELECT p.workspace_id AS legacy_project_id, p.workspace_id, p.name, p.owner_user_id, rd.email AS owner_email, rd.suggested_v2_role AS owner_role_suggestion, CASE WHEN rd.decision_class = 'AUTO_CANDIDATE' AND rd.suggested_v2_role = 'writer' THEN 'PRESERVE_AS_WRITER_PERSONAL' WHEN rd.suggested_v2_role IN ('mentor', 'admin') THEN 'TRANSFER_REQUIRED' ELSE 'REQUIRES_OWNER_ROLE_DECISION' END AS migration_action, CASE WHEN rd.decision_class = 'AUTO_CANDIDATE' AND rd.suggested_v2_role = 'writer' THEN '' ELSE rd.decision_class || ': ' || rd.reason END AS blocker FROM latex_core.projects p JOIN role_decisions rd ON rd.user_id = p.owner_user_id ORDER BY p.workspace_id) TO '/output/personal-papers.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

CREATE TEMP VIEW team_project_counts AS
SELECT t.id AS team_id, count(tp.id) AS project_count
FROM latex_core.teams t
LEFT JOIN latex_core.team_projects tp ON tp.team_id = t.id
GROUP BY t.id;

\copy (SELECT t.id AS legacy_team_id, t.name AS legacy_team_name, tp.id AS legacy_team_project_id, tp.name AS legacy_project_name, tp.workspace_id, CASE WHEN tc.project_count = 1 THEN t.name ELSE t.name || ' — ' || tp.name END AS proposed_v2_team_name, tc.project_count AS legacy_team_project_count, 'CREATE_PAPER_TEAM' AS migration_action, '' AS blocker FROM latex_core.team_projects tp JOIN latex_core.teams t ON t.id = tp.team_id JOIN team_project_counts tc ON tc.team_id = t.id ORDER BY t.id, tp.id) TO '/output/paper-team-plan.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

\copy (SELECT t.id AS legacy_team_id, t.name AS legacy_team_name, t.group_type, t.created_by AS creator_user_id, count(DISTINCT tm.user_id) AS member_count, tc.project_count AS project_count, (tc.project_count > 1) AS is_multi_project, CASE WHEN tc.project_count = 0 THEN 'MANUAL_REVIEW' WHEN tc.project_count > 1 THEN 'SPLIT_PER_PROJECT' ELSE 'MIGRATE_SINGLE_PROJECT' END AS migration_action, CASE WHEN tc.project_count = 0 THEN 'zero-project Team requires ARCHIVE_EMPTY_TEAM or MANUAL_REVIEW decision' ELSE '' END AS blocker FROM latex_core.teams t JOIN team_project_counts tc ON tc.team_id = t.id LEFT JOIN latex_core.team_members tm ON tm.team_id = t.id GROUP BY t.id, t.name, t.group_type, t.created_by, tc.project_count ORDER BY t.id) TO '/output/legacy-teams.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

CREATE TEMP VIEW proposed_team_access AS
SELECT
  tp.team_id,
  tp.id AS team_project_id,
  tp.workspace_id,
  access.user_id,
  bool_or(access.legacy_team_membership) AS legacy_team_membership,
  bool_or(access.writer) AS legacy_writer,
  bool_or(access.mentor) AS legacy_mentor,
  bool_or(access.project_manager) AS legacy_project_manager
FROM latex_core.team_projects tp
CROSS JOIN LATERAL (
  SELECT tm.user_id, true AS legacy_team_membership, false AS writer, false AS mentor, false AS project_manager
  FROM latex_core.team_members tm WHERE tm.team_id = tp.team_id
  UNION ALL
  SELECT pm.user_id, false, pm.writer, pm.mentor, pm.project_manager
  FROM latex_core.team_project_members pm WHERE pm.team_project_id = tp.id
) access
GROUP BY tp.team_id, tp.id, tp.workspace_id, access.user_id;

\copy (SELECT a.team_id AS legacy_team_id, t.name AS legacy_team_name, a.team_project_id AS legacy_project_id, tp.name AS legacy_project_name, a.workspace_id, a.user_id, rd.email, a.legacy_team_membership, a.legacy_writer, a.legacy_mentor, a.legacy_project_manager, rd.suggested_v2_role AS suggested_global_v2_role, 'BLOCKED_ROLE_UNRESOLVED' AS assignment_status, rd.decision_class || ': resolved_v2_role is blank in C2' AS blocker FROM proposed_team_access a JOIN latex_core.teams t ON t.id = a.team_id JOIN latex_core.team_projects tp ON tp.id = a.team_project_id JOIN role_decisions rd ON rd.user_id = a.user_id ORDER BY a.team_id, a.team_project_id, a.user_id) TO '/output/team-memberships.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

CREATE TEMP VIEW event_operations AS
SELECT e.workspace_id, e.sequence, e.created_at, operation
FROM latex_core.workspace_events e
CROSS JOIN LATERAL jsonb_array_elements(COALESCE(e.payload -> 'operations', '[]'::jsonb)) operation;

\copy (SELECT rg.id AS group_id, rg.name AS group_name, rg.owner_user_id, COALESCE(owner.email, '') AS owner_email, string_agg(DISTINCT rgm.user_id::text, ',' ORDER BY rgm.user_id::text) AS member_user_ids, string_agg(DISTINCT COALESCE(member.email, ''), ',' ORDER BY COALESCE(member.email, '')) AS member_emails, count(DISTINCT rgm.user_id) AS member_count, rg.workspace_id, count(DISTINCT eo.operation ->> 'path') FILTER (WHERE eo.operation ->> 'op' = 'put_file') AS observed_file_count, greatest(rg.updated_at, COALESCE(max(eo.created_at), rg.updated_at)) AS latest_activity, 'DECISION_REQUIRED' AS migration_action, 'choose exactly one future action: CONVERT_TO_WRITER_PERSONAL, CONVERT_TO_PAPER_TEAM, or ARCHIVE_EXPORT' AS blocker FROM latex_core.research_groups rg LEFT JOIN latex_core.user_credentials owner ON owner.user_id = rg.owner_user_id LEFT JOIN latex_core.research_group_members rgm ON rgm.group_id = rg.id LEFT JOIN latex_core.user_credentials member ON member.user_id = rgm.user_id LEFT JOIN event_operations eo ON eo.workspace_id = rg.workspace_id GROUP BY rg.id, rg.name, rg.owner_user_id, owner.email, rg.workspace_id, rg.updated_at ORDER BY rg.id) TO '/output/research-groups.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

CREATE TEMP VIEW draft_aggregates AS
SELECT team_project_id, user_id,
       count(*) AS draft_file_count,
       string_agg(logical_path, ',' ORDER BY logical_path) AS draft_paths,
       string_agg(logical_path || ':base=' || base_file_revision, ',' ORDER BY logical_path) AS base_revisions,
       string_agg(logical_path || ':draft=' || draft_revision, ',' ORDER BY logical_path) AS draft_revisions
FROM latex_core.member_drafts
GROUP BY team_project_id, user_id;

CREATE TEMP VIEW operation_aggregates AS
SELECT operations.team_project_id, operations.user_id,
       operations.structural_operation_count,
       operations.structural_operations,
       COALESCE(paths.operation_paths, '') AS operation_paths
FROM (
  SELECT team_project_id, user_id,
         count(*) AS structural_operation_count,
         string_agg(operation_sequence || ':' || operation_type || ':' || COALESCE(source_path, '') || CASE WHEN destination_path IS NULL THEN '' ELSE '->' || destination_path END, ',' ORDER BY operation_sequence) AS structural_operations
  FROM latex_core.member_change_operations
  GROUP BY team_project_id, user_id
) operations
LEFT JOIN (
  SELECT changes.team_project_id, changes.user_id,
         string_agg(DISTINCT paths.path, ',' ORDER BY paths.path) AS operation_paths
  FROM latex_core.member_change_operations changes
  CROSS JOIN LATERAL (VALUES (changes.source_path), (changes.destination_path)) paths(path)
  WHERE paths.path IS NOT NULL
  GROUP BY changes.team_project_id, changes.user_id
) paths ON paths.team_project_id = operations.team_project_id AND paths.user_id = operations.user_id;

CREATE TEMP VIEW private_paths AS
SELECT team_project_id, user_id, string_agg(path, ',' ORDER BY path) AS paths
FROM (
  SELECT team_project_id, user_id, logical_path AS path
  FROM latex_core.member_drafts
  UNION
  SELECT changes.team_project_id, changes.user_id, paths.path
  FROM latex_core.member_change_operations changes
  CROSS JOIN LATERAL (VALUES (changes.source_path), (changes.destination_path)) paths(path)
  WHERE paths.path IS NOT NULL
) all_paths
GROUP BY team_project_id, user_id;

CREATE TEMP VIEW private_work_plan AS
SELECT pcs.team_project_id, pcs.user_id, tp.team_id, t.name AS team_name,
       tp.name AS project_name, tp.workspace_id, tp.canonical_generation,
       COALESCE(da.base_revisions, '') AS base_revisions,
       COALESCE(da.draft_revisions, '') AS draft_revisions,
       COALESCE(da.draft_file_count, 0) AS draft_file_count,
       COALESCE(oa.structural_operation_count, 0) AS structural_operation_count,
       COALESCE(pp.paths, '') AS paths,
       COALESCE(oa.structural_operations, '') AS structural_operations
FROM private_change_sets pcs
JOIN latex_core.team_projects tp ON tp.id = pcs.team_project_id
JOIN latex_core.teams t ON t.id = tp.team_id
LEFT JOIN draft_aggregates da ON da.team_project_id = pcs.team_project_id AND da.user_id = pcs.user_id
LEFT JOIN operation_aggregates oa ON oa.team_project_id = pcs.team_project_id AND oa.user_id = pcs.user_id
LEFT JOIN private_paths pp ON pp.team_project_id = pcs.team_project_id AND pp.user_id = pcs.user_id;

\copy (SELECT pwp.user_id, rd.email, pwp.team_id, pwp.team_name, pwp.team_project_id AS project_id, pwp.project_name, pwp.workspace_id, pwp.canonical_generation AS canonical_revision, pwp.base_revisions, pwp.draft_revisions, pwp.draft_file_count, pwp.structural_operation_count, pwp.paths, pwp.structural_operations, 'UNRESOLVED' AS migration_status FROM private_work_plan pwp JOIN role_decisions rd ON rd.user_id = pwp.user_id ORDER BY pwp.team_project_id, pwp.user_id) TO '/output/private-work.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

\copy (SELECT tm.id AS template_id, tm.name, COALESCE(tm.main_file, '') AS main_file, COALESCE(string_agg(DISTINCT ta.account_type, ',' ORDER BY ta.account_type), '') AS audience, 'legacy_unversioned; created_at=' || tm.created_at || '; linked_team_projects=' || count(DISTINCT tp.id) || '; direct_user_grants=' || count(DISTINCT tug.user_id) AS provenance_version_signals, count(DISTINCT tf.path) AS file_count, CASE WHEN tm.main_file IS NULL OR EXISTS (SELECT 1 FROM latex_core.template_files main_tf WHERE main_tf.template_id = tm.id AND main_tf.path = tm.main_file) THEN 'PRESERVE_TEMPLATE_SOURCE' ELSE 'MANUAL_REQUIRED' END AS future_v2_action, CASE WHEN tm.main_file IS NOT NULL AND NOT EXISTS (SELECT 1 FROM latex_core.template_files main_tf WHERE main_tf.template_id = tm.id AND main_tf.path = tm.main_file) THEN 'declared main file is absent from template files' ELSE '' END AS blocker FROM latex_core.templates tm LEFT JOIN latex_core.template_account_types ta ON ta.template_id = tm.id LEFT JOIN latex_core.template_files tf ON tf.template_id = tm.id LEFT JOIN latex_core.template_user_grants tug ON tug.template_id = tm.id LEFT JOIN latex_core.team_projects tp ON tp.template_id = tm.id GROUP BY tm.id, tm.name, tm.main_file, tm.created_at ORDER BY tm.id) TO '/output/templates.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

CREATE TEMP VIEW policy_plan AS
SELECT
  tp.team_id AS legacy_team_id,
  t.name AS legacy_team_name,
  fp.team_project_id AS legacy_project_id,
  tp.name AS legacy_project_name,
  fp.logical_path AS legacy_path,
  fp.access_policy AS legacy_policy,
  fp.origin AS legacy_source,
  CASE fp.access_policy
    WHEN 'editable' THEN 'EDITABLE'
    WHEN 'read_only' THEN 'CONTENT_READ_ONLY'
    WHEN 'managed' THEN 'TEMPLATE_MANAGED'
    ELSE ''
  END AS suggested_v2_policy,
  CASE
    WHEN fp.access_policy IN ('editable', 'read_only') THEN 'DETERMINISTIC'
    WHEN fp.access_policy = 'managed' AND fp.origin = 'template' THEN 'DETERMINISTIC'
    ELSE 'MANUAL_REQUIRED'
  END AS decision_class,
  CASE
    WHEN fp.access_policy = 'editable' THEN 'legacy Writer content and structure were editable'
    WHEN fp.access_policy = 'read_only' THEN 'legacy ordinary client content and structure were read-only'
    WHEN fp.access_policy = 'managed' AND fp.origin = 'template' THEN 'legacy managed protection has explicit template provenance'
    WHEN fp.access_policy = 'managed' THEN 'legacy managed protection was set outside template provenance and relied on a removed Project Manager capability'
    ELSE 'unknown legacy policy has no proven V2 equivalent'
  END AS reason
FROM latex_core.file_policies fp
JOIN latex_core.team_projects tp ON tp.id = fp.team_project_id
JOIN latex_core.teams t ON t.id = tp.team_id;

\copy (SELECT legacy_team_id, legacy_team_name, legacy_project_id, legacy_project_name, legacy_path, legacy_policy, legacy_source, suggested_v2_policy, decision_class, reason FROM policy_plan ORDER BY legacy_team_id, legacy_project_id, legacy_path) TO '/output/file-policy-plan.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

CREATE TEMP VIEW unresolved_plan AS
SELECT CASE WHEN rd.decision_class = 'BLOCKED_MISSING_ACCOUNT' THEN 'MISSING_ACCOUNT' ELSE 'USER_ROLE' END::text AS category,
       'user'::text AS entity_type, rd.user_id::text AS entity_id,
       COALESCE(NULLIF(rd.email, ''), rd.user_id::text) AS identity_hint,
       rd.reason, 'record a validated exclusive V2 role or archive/exclusion decision'::text AS required_future_action
FROM role_decisions rd WHERE rd.decision_class <> 'AUTO_CANDIDATE'
UNION ALL
SELECT 'ROLE_CONFLICT', 'user', rd.user_id::text, COALESCE(NULLIF(rd.email, ''), rd.user_id::text),
       'legacy project membership contains overlapping Writer/Mentor/Project Manager roles',
       'resolve the global role and compatible paper assignments'
FROM role_decisions rd WHERE rd.has_role_overlap
UNION ALL
SELECT 'PERSONAL_PAPER_OWNER', 'personal_paper', p.workspace_id::text, p.name,
       'owner is not yet safely validated as a V2 Writer',
       'select transfer, Paper Team conversion, archive, or a compatible Writer role'
FROM latex_core.projects p JOIN role_decisions rd ON rd.user_id = p.owner_user_id
WHERE rd.decision_class <> 'AUTO_CANDIDATE' OR rd.suggested_v2_role <> 'writer'
UNION ALL
SELECT 'RESEARCH_GROUP', 'research_group', rg.id::text, rg.name,
       'Research Groups have no V2 equivalent',
       'select CONVERT_TO_WRITER_PERSONAL, CONVERT_TO_PAPER_TEAM, or ARCHIVE_EXPORT'
FROM latex_core.research_groups rg
UNION ALL
SELECT 'PRIVATE_WORK', 'project_user_change_set', pwp.team_project_id::text || ':' || pwp.user_id::text,
       pwp.project_name || ' / ' || pwp.user_id::text,
       'unpublished private draft files or structural operations remain',
       'record merge, export, archive, or explicit rejection disposition'
FROM private_work_plan pwp
UNION ALL
SELECT 'POLICY_MAPPING', 'file_policy', pp.legacy_project_id::text || ':' || pp.legacy_path,
       pp.legacy_project_name || ' / ' || pp.legacy_path,
       pp.reason, 'record the intended V2 file policy in the future decision manifest'
FROM policy_plan pp WHERE pp.decision_class = 'MANUAL_REQUIRED'
UNION ALL
SELECT 'ZERO_PROJECT_TEAM', 'team', t.id::text, t.name,
       'legacy Team has no project or paper workspace',
       'select ARCHIVE_EMPTY_TEAM or MANUAL_REVIEW disposition'
FROM latex_core.teams t JOIN team_project_counts tc ON tc.team_id = t.id WHERE tc.project_count = 0;

\copy (SELECT category, entity_type, entity_id, identity_hint, reason, required_future_action FROM unresolved_plan ORDER BY category, entity_type, entity_id, reason) TO '/output/unresolved.tsv' WITH (FORMAT csv, DELIMITER E'\t', HEADER true)

\pset tuples_only on
\pset format unaligned
\o /output/migration-plan.json
SELECT jsonb_pretty(jsonb_build_object(
  'source_baseline_commit', :'source_commit',
  'source_dump_sha256', :'dump_sha256',
  'target_architecture_commit', :'target_commit',
  'counts', (SELECT jsonb_object_agg(metric, actual ORDER BY ordinal) FROM reconciliation),
  'paper_team_candidates', (SELECT count(*) FROM latex_core.team_projects),
  'personal_paper_candidates', (SELECT count(*) FROM latex_core.projects),
  'automatic_role_candidates', (SELECT count(*) FROM role_decisions WHERE decision_class = 'AUTO_CANDIDATE'),
  'manual_role_decisions', (SELECT count(*) FROM role_decisions WHERE decision_class = 'MANUAL_REQUIRED'),
  'missing_account_blockers', (SELECT count(*) FROM role_decisions WHERE decision_class = 'BLOCKED_MISSING_ACCOUNT'),
  'private_work_blockers', (SELECT count(*) FROM private_change_sets),
  'research_group_decisions', (SELECT count(*) FROM latex_core.research_groups),
  'policy_decisions', (SELECT count(*) FROM policy_plan WHERE decision_class = 'MANUAL_REQUIRED'),
  'unresolved_total', (SELECT count(*) FROM unresolved_plan),
  'ready_for_destructive_migration', false
));
\o

\o /output/summary.txt
SELECT 'V2 Migration Planning Summary';
SELECT 'Source baseline commit: ' || :'source_commit';
SELECT 'Source dump SHA-256: ' || :'dump_sha256';
SELECT 'Target architecture commit: ' || :'target_commit';
SELECT 'Reconciliation: 20/20 MATCH';
SELECT 'Paper Team candidates: ' || count(*) FROM latex_core.team_projects;
SELECT 'Personal paper candidates: ' || count(*) FROM latex_core.projects;
SELECT 'Automatic role candidates: ' || count(*) FROM role_decisions WHERE decision_class = 'AUTO_CANDIDATE';
SELECT 'Manual role decisions: ' || count(*) FROM role_decisions WHERE decision_class = 'MANUAL_REQUIRED';
SELECT 'Missing-account blockers: ' || count(*) FROM role_decisions WHERE decision_class = 'BLOCKED_MISSING_ACCOUNT';
SELECT 'Private-work blockers: ' || count(*) FROM private_change_sets;
SELECT 'Research Group decisions: ' || count(*) FROM latex_core.research_groups;
SELECT 'Policy decisions: ' || count(*) FROM policy_plan WHERE decision_class = 'MANUAL_REQUIRED';
SELECT 'Unresolved entries: ' || count(*) FROM unresolved_plan;
SELECT 'Ready for destructive migration: false';
\o

\o /output/README.txt
SELECT 'V2 MIGRATION PLANNER OUTPUT';
SELECT '';
SELECT 'This directory contains private migration metadata generated from the frozen C0 snapshot.';
SELECT 'It contains identifiers and emails but no passwords, credential hashes, sessions, cookies, blob hashes, or source-file contents.';
SELECT '';
SELECT 'C2 IS A READ-ONLY PLAN. It does not migrate, merge, transfer, archive, discard, or otherwise mutate legacy data.';
SELECT 'resolved_v2_role is intentionally blank. Private work is intentionally UNRESOLVED.';
SELECT 'Use migration-plan.json for machine-readable readiness and unresolved.tsv for future decision work.';
SELECT 'Do not commit this output directory to Git.';
\o
