//! Narrow machine-authenticated institutional read projections.

use crate::{Database, GlobalRole};
use core_types::{BlobHash, UserId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use std::{collections::BTreeSet, str::FromStr};
use thiserror::Error;
use uuid::Uuid;

pub const INTEGRATION_SCOPES: &[&str] = &[
    "institution.directory.read",
    "institution.contacts.read",
    "reports.read",
    "reports.files.read",
    "reports.pdf.read",
    "reviews.published.read",
];

#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("integration client not found")]
    NotFound,
    #[error("integration request is not authorized")]
    Forbidden,
    #[error("integration rate limit exceeded")]
    RateLimited,
    #[error("invalid integration client: {0}")]
    Invalid(String),
    #[error("integration persistence failed")]
    Database(#[source] sqlx::Error),
    #[error("invalid stored integration data: {0}")]
    Integrity(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntegrationClient {
    pub id: Uuid,
    pub name: String,
    pub token_prefix: String,
    pub scopes: Vec<String>,
    pub institution_wide: bool,
    pub report_ids: Vec<Uuid>,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub created_at: String,
    pub rotated_at: Option<String>,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug)]
pub struct IntegrationPrincipal {
    pub client_id: Uuid,
    pub name: String,
    pub scopes: BTreeSet<String>,
    pub institution_wide: bool,
    pub report_ids: BTreeSet<Uuid>,
}

impl IntegrationPrincipal {
    #[must_use]
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }

    #[must_use]
    pub fn allows_report(&self, report_id: Uuid) -> bool {
        self.institution_wide || self.report_ids.contains(&report_id)
    }
}

#[derive(Clone, Debug)]
pub struct IntegrationRepository {
    database: Database,
}

impl IntegrationRepository {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_client(
        &self,
        admin: UserId,
        name: &str,
        token_prefix: &str,
        token_hash: &[u8],
        scopes: &[String],
        institution_wide: bool,
        report_ids: &[Uuid],
        expires_at: Option<&str>,
    ) -> Result<IntegrationClient, IntegrationError> {
        validate_client(name, scopes, institution_wide, report_ids)?;
        require_admin(self.database.pool(), admin).await?;
        validate_report_ids(self.database.pool(), report_ids).await?;
        let row = sqlx::query(
            "INSERT INTO latex_core.integration_clients \
             (id,name,token_prefix,token_hash,scopes,institution_wide,report_ids,expires_at,created_by_admin_user_id) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8::timestamptz,$9) \
             RETURNING id,name,token_prefix,scopes,institution_wide,report_ids,expires_at::text,revoked_at::text,created_at::text,rotated_at::text,last_used_at::text",
        )
        .bind(Uuid::new_v4())
        .bind(name.trim())
        .bind(token_prefix)
        .bind(token_hash)
        .bind(scopes)
        .bind(institution_wide)
        .bind(report_ids)
        .bind(expires_at)
        .bind(admin.as_uuid())
        .fetch_one(self.database.pool())
        .await
        .map_err(IntegrationError::Database)?;
        decode_client(&row)
    }

    pub async fn list_clients(
        &self,
        admin: UserId,
    ) -> Result<Vec<IntegrationClient>, IntegrationError> {
        require_admin(self.database.pool(), admin).await?;
        let rows = sqlx::query(
            "SELECT id,name,token_prefix,scopes,institution_wide,report_ids,expires_at::text,revoked_at::text,created_at::text,rotated_at::text,last_used_at::text \
             FROM latex_core.integration_clients ORDER BY created_at,id LIMIT 500",
        )
        .fetch_all(self.database.pool())
        .await
        .map_err(IntegrationError::Database)?;
        rows.iter().map(decode_client).collect()
    }

    pub async fn revoke_client(&self, admin: UserId, id: Uuid) -> Result<(), IntegrationError> {
        require_admin(self.database.pool(), admin).await?;
        let changed = sqlx::query(
            "UPDATE latex_core.integration_clients SET revoked_at=COALESCE(revoked_at,statement_timestamp()) WHERE id=$1",
        )
        .bind(id)
        .execute(self.database.pool())
        .await
        .map_err(IntegrationError::Database)?;
        if changed.rows_affected() == 0 {
            Err(IntegrationError::NotFound)
        } else {
            Ok(())
        }
    }

    pub async fn rotate_client(
        &self,
        admin: UserId,
        id: Uuid,
        token_prefix: &str,
        token_hash: &[u8],
    ) -> Result<IntegrationClient, IntegrationError> {
        require_admin(self.database.pool(), admin).await?;
        let row = sqlx::query(
            "UPDATE latex_core.integration_clients SET token_prefix=$2,token_hash=$3,revoked_at=NULL,rotated_at=statement_timestamp() WHERE id=$1 \
             RETURNING id,name,token_prefix,scopes,institution_wide,report_ids,expires_at::text,revoked_at::text,created_at::text,rotated_at::text,last_used_at::text",
        )
        .bind(id)
        .bind(token_prefix)
        .bind(token_hash)
        .fetch_optional(self.database.pool())
        .await
        .map_err(IntegrationError::Database)?
        .ok_or(IntegrationError::NotFound)?;
        decode_client(&row)
    }

    pub async fn authenticate(
        &self,
        token_hash: &[u8],
    ) -> Result<IntegrationPrincipal, IntegrationError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(IntegrationError::Database)?;
        let row = sqlx::query(
            "SELECT id,name,scopes,institution_wide,report_ids FROM latex_core.integration_clients \
             WHERE token_hash=$1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>statement_timestamp()) FOR UPDATE",
        )
        .bind(token_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(IntegrationError::Database)?
        .ok_or(IntegrationError::Forbidden)?;
        let client_id: Uuid = row.try_get("id").map_err(IntegrationError::Database)?;
        sqlx::query("UPDATE latex_core.integration_clients SET last_used_at=statement_timestamp() WHERE id=$1")
            .bind(client_id).execute(&mut *tx).await.map_err(IntegrationError::Database)?;
        tx.commit().await.map_err(IntegrationError::Database)?;
        Ok(IntegrationPrincipal {
            client_id,
            name: row.try_get("name").map_err(IntegrationError::Database)?,
            scopes: row
                .try_get::<Vec<String>, _>("scopes")
                .map_err(IntegrationError::Database)?
                .into_iter()
                .collect(),
            institution_wide: row
                .try_get("institution_wide")
                .map_err(IntegrationError::Database)?,
            report_ids: row
                .try_get::<Vec<Uuid>, _>("report_ids")
                .map_err(IntegrationError::Database)?
                .into_iter()
                .collect(),
        })
    }

    /// Admit one authenticated read request and retain a deliberately minimal audit row.
    /// The database-backed window makes revocation and rate limiting consistent across API
    /// processes without logging credentials, query values, or returned institutional data.
    pub async fn admit_read(
        &self,
        client_id: Uuid,
        route_name: &str,
    ) -> Result<(), IntegrationError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(IntegrationError::Database)?;
        sqlx::query("SELECT id FROM latex_core.integration_clients WHERE id=$1 FOR UPDATE")
            .bind(client_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(IntegrationError::Database)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.integration_access_log \
             WHERE client_id=$1 AND outcome='ALLOWED' AND occurred_at>=statement_timestamp()-interval '1 minute'",
        )
        .bind(client_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(IntegrationError::Database)?;
        if count >= 120 {
            return Err(IntegrationError::RateLimited);
        }
        sqlx::query(
            "INSERT INTO latex_core.integration_access_log \
             (client_id,method,route_name,outcome) VALUES ($1,'GET',$2,'ALLOWED')",
        )
        .bind(client_id)
        .bind(route_name)
        .execute(&mut *tx)
        .await
        .map_err(IntegrationError::Database)?;
        tx.commit().await.map_err(IntegrationError::Database)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn reports(
        &self,
        principal: &IntegrationPrincipal,
        after: Option<Uuid>,
        limit: i64,
        programme: Option<&str>,
        academic_year: Option<&str>,
        semester: Option<&str>,
        state: Option<&str>,
    ) -> Result<Vec<Value>, IntegrationError> {
        let ids = principal.report_ids.iter().copied().collect::<Vec<_>>();
        let rows = sqlx::query(
            "SELECT t.id,t.name,t.status,t.created_at::text,t.updated_at::text,g.academic_year,g.semester, \
                    COALESCE(fm.dominant_programme_code,tr.dominant_programme_code) AS programme_code, \
                    pin.template_id,template.name AS template_name,fm.front_matter_pack_id,pack.name AS front_matter_pack_name, \
                    EXISTS(SELECT 1 FROM latex_core.review_rounds rr WHERE rr.paper_id=t.id AND rr.status='OPEN_FOR_REVIEW') AS review_open \
             FROM latex_core.paper_teams t \
             LEFT JOIN latex_core.external_paper_team_links link ON link.paper_team_id=t.id \
             LEFT JOIN vcap.paper_assignment_groups g ON g.external_team_key=link.external_team_key \
             LEFT JOIN latex_core.paper_template_resolutions tr ON tr.paper_team_id=t.id \
             LEFT JOIN latex_core.paper_template_pins pin ON pin.paper_id=t.id \
             LEFT JOIN latex_core.templates template ON template.id=pin.template_id \
             LEFT JOIN latex_core.paper_front_matter_pins fm ON fm.paper_team_id=t.id \
             LEFT JOIN latex_core.front_matter_packs pack ON pack.id=fm.front_matter_pack_id \
             WHERE ($1 OR t.id=ANY($2)) AND ($3::uuid IS NULL OR t.id>$3) \
               AND ($4::text IS NULL OR COALESCE(fm.dominant_programme_code,tr.dominant_programme_code)=$4) \
               AND ($5::text IS NULL OR g.academic_year=$5) AND ($6::text IS NULL OR g.semester=$6) \
               AND ($7::text IS NULL OR t.status=$7) ORDER BY t.id LIMIT $8",
        )
        .bind(principal.institution_wide).bind(&ids).bind(after).bind(programme)
        .bind(academic_year).bind(semester).bind(state).bind(limit)
        .fetch_all(self.database.pool()).await.map_err(IntegrationError::Database)?;
        rows.iter().map(report_summary).collect()
    }

    pub async fn report(&self, report_id: Uuid, contacts: bool) -> Result<Value, IntegrationError> {
        let row = sqlx::query(
            "SELECT t.id,t.name,t.status,t.created_at::text,t.updated_at::text,g.academic_year,g.semester, \
                    COALESCE(fm.dominant_programme_code,tr.dominant_programme_code) AS programme_code, \
                    pin.template_id,template.name AS template_name,fm.front_matter_pack_id,pack.name AS front_matter_pack_name, \
                    EXISTS(SELECT 1 FROM latex_core.review_rounds rr WHERE rr.paper_id=t.id AND rr.status='OPEN_FOR_REVIEW') AS review_open \
             FROM latex_core.paper_teams t LEFT JOIN latex_core.external_paper_team_links link ON link.paper_team_id=t.id \
             LEFT JOIN vcap.paper_assignment_groups g ON g.external_team_key=link.external_team_key \
             LEFT JOIN latex_core.paper_template_resolutions tr ON tr.paper_team_id=t.id \
             LEFT JOIN latex_core.paper_template_pins pin ON pin.paper_id=t.id LEFT JOIN latex_core.templates template ON template.id=pin.template_id \
             LEFT JOIN latex_core.paper_front_matter_pins fm ON fm.paper_team_id=t.id LEFT JOIN latex_core.front_matter_packs pack ON pack.id=fm.front_matter_pack_id \
             WHERE t.id=$1",
        ).bind(report_id).fetch_optional(self.database.pool()).await.map_err(IntegrationError::Database)?.ok_or(IntegrationError::NotFound)?;
        let mut report = report_summary(&row)?;
        let members = sqlx::query(
            "SELECT m.user_id,m.is_leader,m.writer_order,r.role,c.email,s.reg_no,s.name AS student_name,f.faculty_id,f.name AS faculty_name \
             FROM latex_core.paper_team_members m JOIN latex_core.global_user_roles r ON r.user_id=m.user_id \
             JOIN latex_core.user_credentials c ON c.user_id=m.user_id \
             LEFT JOIN vcap.student_user_links sl ON sl.user_id=m.user_id AND sl.status='LINKED' LEFT JOIN vcap.students s ON s.reg_no=sl.reg_no \
             LEFT JOIN vcap.faculty_user_links fl ON fl.user_id=m.user_id AND fl.status='LINKED' LEFT JOIN vcap.faculty f ON f.faculty_id=fl.faculty_id \
             WHERE m.paper_team_id=$1 ORDER BY CASE WHEN r.role='writer' THEN 0 ELSE 1 END,m.writer_order NULLS LAST,m.created_at,m.user_id",
        ).bind(report_id).fetch_all(self.database.pool()).await.map_err(IntegrationError::Database)?;
        let people = members.iter().map(|member| {
            let role: String = member.try_get("role").map_err(IntegrationError::Database)?;
            let mut value = json!({
                "user_id":member.try_get::<Uuid,_>("user_id").map_err(IntegrationError::Database)?,
                "role":role,"is_leader":member.try_get::<bool,_>("is_leader").map_err(IntegrationError::Database)?,
                "writer_order":member.try_get::<Option<i32>,_>("writer_order").map_err(IntegrationError::Database)?,
                "institutional_id":if role=="writer" { member.try_get::<Option<String>,_>("reg_no").map_err(IntegrationError::Database)? } else { member.try_get::<Option<String>,_>("faculty_id").map_err(IntegrationError::Database)? },
                "display_name":if role=="writer" { member.try_get::<Option<String>,_>("student_name").map_err(IntegrationError::Database)? } else { member.try_get::<Option<String>,_>("faculty_name").map_err(IntegrationError::Database)? },
            });
            if contacts { value["email"] = json!(member.try_get::<String,_>("email").map_err(IntegrationError::Database)?); }
            Ok(value)
        }).collect::<Result<Vec<_>,IntegrationError>>()?;
        report["relationships"] = json!({"members":people});
        Ok(report)
    }

    pub async fn report_exists(&self, report_id: Uuid) -> Result<bool, IntegrationError> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.paper_teams WHERE id=$1)")
            .bind(report_id)
            .fetch_one(self.database.pool())
            .await
            .map_err(IntegrationError::Database)
    }

    pub async fn versions(
        &self,
        report_id: Uuid,
        after: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<Value>, IntegrationError> {
        let rows = sqlx::query(
            "SELECT paper_id,id,document_epoch,version_number,version_type,name,workspace_version,snapshot_id,state_hash,manifest,created_at::text \
             FROM latex_core.paper_versions WHERE paper_id=$1 AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3",
        ).bind(report_id).bind(after).bind(limit).fetch_all(self.database.pool()).await.map_err(IntegrationError::Database)?;
        rows.iter().map(version_json).collect()
    }

    pub async fn version(
        &self,
        report_id: Uuid,
        version_id: Uuid,
    ) -> Result<Value, IntegrationError> {
        let row = sqlx::query(
            "SELECT paper_id,id,document_epoch,version_number,version_type,name,workspace_version,snapshot_id,state_hash,manifest,created_at::text \
             FROM latex_core.paper_versions WHERE paper_id=$1 AND id=$2",
        ).bind(report_id).bind(version_id).fetch_optional(self.database.pool()).await.map_err(IntegrationError::Database)?.ok_or(IntegrationError::NotFound)?;
        version_json(&row)
    }

    pub async fn pdf(&self, report_id: Uuid, build_id: Uuid) -> Result<Value, IntegrationError> {
        let row = sqlx::query(
            "SELECT b.id,b.version_id,b.source_sequence,b.state_hash,b.created_at::text,a.blob_hash,a.size_bytes,a.content_type, \
                    (s.current_build_id=b.id AND h.durable_version=b.source_sequence) AS is_current \
             FROM latex_core.v2_paper_builds b JOIN latex_core.compilation_artifacts a ON a.job_id=b.compile_job_id AND a.kind='pdf' \
             JOIN latex_core.workspace_heads h ON h.workspace_id=b.workspace_id LEFT JOIN latex_core.v2_paper_build_state s ON s.workspace_id=b.workspace_id \
             WHERE b.paper_id=$1 AND b.id=$2 AND b.status='succeeded' ORDER BY a.logical_name LIMIT 1",
        ).bind(report_id).bind(build_id).fetch_optional(self.database.pool()).await.map_err(IntegrationError::Database)?.ok_or(IntegrationError::NotFound)?;
        Ok(
            json!({"build_id":row.try_get::<Uuid,_>("id").map_err(IntegrationError::Database)?,"version_id":row.try_get::<Uuid,_>("version_id").map_err(IntegrationError::Database)?,"source_sequence":row.try_get::<i64,_>("source_sequence").map_err(IntegrationError::Database)?,"state_hash":row.try_get::<String,_>("state_hash").map_err(IntegrationError::Database)?,"blob_hash":row.try_get::<String,_>("blob_hash").map_err(IntegrationError::Database)?,"size_bytes":row.try_get::<i64,_>("size_bytes").map_err(IntegrationError::Database)?,"content_type":row.try_get::<String,_>("content_type").map_err(IntegrationError::Database)?,"is_current":row.try_get::<bool,_>("is_current").map_err(IntegrationError::Database)?,"created_at":row.try_get::<String,_>("created_at").map_err(IntegrationError::Database)?}),
        )
    }

    pub async fn builds(
        &self,
        report_id: Uuid,
        after: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<Value>, IntegrationError> {
        let rows = sqlx::query(
            "SELECT b.id,b.version_id,b.source_sequence,b.state_hash,b.created_at::text,a.blob_hash,a.size_bytes, \
                    (s.current_build_id=b.id AND h.durable_version=b.source_sequence) AS is_current \
             FROM latex_core.v2_paper_builds b \
             JOIN latex_core.compilation_artifacts a ON a.job_id=b.compile_job_id AND a.kind='pdf' \
             JOIN latex_core.workspace_heads h ON h.workspace_id=b.workspace_id \
             LEFT JOIN latex_core.v2_paper_build_state s ON s.workspace_id=b.workspace_id \
             WHERE b.paper_id=$1 AND b.status='succeeded' AND ($2::uuid IS NULL OR b.id>$2) \
             ORDER BY b.id LIMIT $3",
        )
        .bind(report_id)
        .bind(after)
        .bind(limit)
        .fetch_all(self.database.pool())
        .await
        .map_err(IntegrationError::Database)?;
        rows.iter()
            .map(|row| {
                let id: Uuid = row.try_get("id").map_err(IntegrationError::Database)?;
                Ok(json!({
                    "id":id,
                    "report_id":report_id,
                    "version_id":row.try_get::<Uuid,_>("version_id").map_err(IntegrationError::Database)?,
                    "source_sequence":row.try_get::<i64,_>("source_sequence").map_err(IntegrationError::Database)?,
                    "state_hash":row.try_get::<String,_>("state_hash").map_err(IntegrationError::Database)?,
                    "sha256":row.try_get::<String,_>("blob_hash").map_err(IntegrationError::Database)?,
                    "size_bytes":row.try_get::<i64,_>("size_bytes").map_err(IntegrationError::Database)?,
                    "is_current":row.try_get::<bool,_>("is_current").map_err(IntegrationError::Database)?,
                    "created_at":row.try_get::<String,_>("created_at").map_err(IntegrationError::Database)?,
                    "download_url":format!("/api/integration/v1/reports/{report_id}/builds/{id}/pdf")
                }))
            })
            .collect()
    }

    pub async fn directory(
        &self,
        resource: &str,
        after: Option<&str>,
        limit: i64,
        contacts: bool,
    ) -> Result<Vec<Value>, IntegrationError> {
        let (query, id_column) = match resource {
            "students" => (
                "SELECT reg_no AS id,name,email,programme_code,NULL::text AS extra FROM vcap.students WHERE ($1::text IS NULL OR reg_no>$1) ORDER BY reg_no LIMIT $2",
                "reg_no",
            ),
            "faculty" => (
                "SELECT faculty_id AS id,name,email,dept_id::text AS programme_code,designation AS extra FROM vcap.faculty WHERE ($1::text IS NULL OR faculty_id>$1) ORDER BY faculty_id LIMIT $2",
                "faculty_id",
            ),
            "programmes" => (
                "SELECT programme_code AS id,NULL::text AS name,NULL::text AS email,NULL::text AS programme_code,hod_id AS extra FROM vcap.programmes WHERE ($1::text IS NULL OR programme_code>$1) ORDER BY programme_code LIMIT $2",
                "programme_code",
            ),
            "departments" => (
                "SELECT department_id::text AS id,NULL::text AS name,NULL::text AS email,NULL::text AS programme_code,NULL::text AS extra FROM vcap.departments WHERE ($1::text IS NULL OR department_id::text>$1) ORDER BY department_id::text LIMIT $2",
                "department_id",
            ),
            "schools" => (
                "SELECT school_id AS id,NULL::text AS name,NULL::text AS email,NULL::text AS programme_code,NULL::text AS extra FROM vcap.schools WHERE ($1::text IS NULL OR school_id>$1) ORDER BY school_id LIMIT $2",
                "school_id",
            ),
            "course-registrations" => (
                "SELECT student_reg_no||':'||course_id||':'||academic_year||':'||semester AS id,course_id AS name,NULL::text AS email,student_reg_no AS programme_code,academic_year||' / '||semester||COALESCE(' / '||registration_status,'') AS extra FROM vcap.student_course_registrations WHERE ($1::text IS NULL OR student_reg_no||':'||course_id||':'||academic_year||':'||semester>$1) ORDER BY id LIMIT $2",
                "course_registration",
            ),
            "faculty-roles" => (
                "SELECT role_id::text AS id,faculty_id AS name,NULL::text AS email,COALESCE(programme_code,department_id::text,school_id) AS programme_code,role_type AS extra FROM vcap.faculty_roles WHERE ($1::text IS NULL OR role_id::text>$1) ORDER BY role_id::text LIMIT $2",
                "faculty_role",
            ),
            "department-roles" => (
                "SELECT id::text AS id,faculty_id AS name,NULL::text AS email,dept_id AS programme_code,role_type AS extra FROM vcap.department_roles WHERE ($1::text IS NULL OR id::text>$1) ORDER BY id LIMIT $2",
                "department_role",
            ),
            _ => return Err(IntegrationError::NotFound),
        };
        let rows = sqlx::query(query)
            .bind(after)
            .bind(limit)
            .fetch_all(self.database.pool())
            .await
            .map_err(IntegrationError::Database)?;
        rows.iter().map(|row| {
            let mut value = json!({"id":row.try_get::<String,_>("id").map_err(IntegrationError::Database)?,"display_name":row.try_get::<Option<String>,_>("name").map_err(IntegrationError::Database)?,"programme_or_department":row.try_get::<Option<String>,_>("programme_code").map_err(IntegrationError::Database)?,"role_or_head":row.try_get::<Option<String>,_>("extra").map_err(IntegrationError::Database)?});
            if contacts { value["email"] = json!(row.try_get::<Option<String>,_>("email").map_err(IntegrationError::Database)?); }
            value["identity_kind"] = json!(id_column);
            Ok(value)
        }).collect()
    }

    pub async fn published_reviews(
        &self,
        report_id: Uuid,
        after: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<Value>, IntegrationError> {
        let rows = sqlx::query(
            "SELECT rt.id,rt.review_round_id,rr.round_number,rt.thread_type,rt.state,rt.severity,rt.category,rt.section_label,rt.published_at::text, \
                    COALESCE((SELECT jsonb_agg(jsonb_build_object('body',m.body,'created_at',m.created_at) ORDER BY m.created_at,m.id) FROM latex_core.review_messages m WHERE m.thread_id=rt.id),'[]') AS messages \
             FROM latex_core.review_threads rt JOIN latex_core.review_rounds rr ON rr.id=rt.review_round_id \
             WHERE rr.paper_id=$1 AND rt.publication_status='PUBLISHED' AND ($2::uuid IS NULL OR rt.id>$2) ORDER BY rt.id LIMIT $3",
        ).bind(report_id).bind(after).bind(limit).fetch_all(self.database.pool()).await.map_err(IntegrationError::Database)?;
        rows.iter().map(|row| Ok(json!({"id":row.try_get::<Uuid,_>("id").map_err(IntegrationError::Database)?,"review_round_id":row.try_get::<Uuid,_>("review_round_id").map_err(IntegrationError::Database)?,"round_number":row.try_get::<i64,_>("round_number").map_err(IntegrationError::Database)?,"type":row.try_get::<String,_>("thread_type").map_err(IntegrationError::Database)?,"state":row.try_get::<String,_>("state").map_err(IntegrationError::Database)?,"severity":row.try_get::<String,_>("severity").map_err(IntegrationError::Database)?,"category":row.try_get::<String,_>("category").map_err(IntegrationError::Database)?,"section_label":row.try_get::<Option<String>,_>("section_label").map_err(IntegrationError::Database)?,"published_at":row.try_get::<Option<String>,_>("published_at").map_err(IntegrationError::Database)?,"messages":row.try_get::<Value,_>("messages").map_err(IntegrationError::Database)?}))).collect()
    }
}

fn validate_client(
    name: &str,
    scopes: &[String],
    institution_wide: bool,
    report_ids: &[Uuid],
) -> Result<(), IntegrationError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(IntegrationError::Invalid(
            "name must contain 1 to 120 characters".into(),
        ));
    }
    let unique = scopes.iter().collect::<BTreeSet<_>>();
    if scopes.is_empty()
        || unique.len() != scopes.len()
        || scopes
            .iter()
            .any(|scope| !INTEGRATION_SCOPES.contains(&scope.as_str()))
    {
        return Err(IntegrationError::Invalid(
            "scopes must be unique supported read scopes".into(),
        ));
    }
    if !institution_wide && report_ids.is_empty() {
        return Err(IntegrationError::Invalid(
            "report coverage is required unless institution_wide is granted".into(),
        ));
    }
    Ok(())
}

async fn validate_report_ids(
    pool: &sqlx::PgPool,
    report_ids: &[Uuid],
) -> Result<(), IntegrationError> {
    if report_ids.is_empty() {
        return Ok(());
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM latex_core.paper_teams WHERE id=ANY($1)")
            .bind(report_ids)
            .fetch_one(pool)
            .await
            .map_err(IntegrationError::Database)?;
    if usize::try_from(count).ok()
        == Some(report_ids.iter().copied().collect::<BTreeSet<_>>().len())
    {
        Ok(())
    } else {
        Err(IntegrationError::Invalid(
            "report coverage contains an unknown Team report".into(),
        ))
    }
}

async fn require_admin(pool: &sqlx::PgPool, admin: UserId) -> Result<(), IntegrationError> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM latex_core.global_user_roles WHERE user_id=$1")
            .bind(admin.as_uuid())
            .fetch_optional(pool)
            .await
            .map_err(IntegrationError::Database)?;
    if role.as_deref() == Some(GlobalRole::Admin.as_str()) {
        Ok(())
    } else {
        Err(IntegrationError::Forbidden)
    }
}

fn decode_client(row: &sqlx::postgres::PgRow) -> Result<IntegrationClient, IntegrationError> {
    Ok(IntegrationClient {
        id: row.try_get("id").map_err(IntegrationError::Database)?,
        name: row.try_get("name").map_err(IntegrationError::Database)?,
        token_prefix: row
            .try_get("token_prefix")
            .map_err(IntegrationError::Database)?,
        scopes: row.try_get("scopes").map_err(IntegrationError::Database)?,
        institution_wide: row
            .try_get("institution_wide")
            .map_err(IntegrationError::Database)?,
        report_ids: row
            .try_get("report_ids")
            .map_err(IntegrationError::Database)?,
        expires_at: row
            .try_get("expires_at")
            .map_err(IntegrationError::Database)?,
        revoked_at: row
            .try_get("revoked_at")
            .map_err(IntegrationError::Database)?,
        created_at: row
            .try_get("created_at")
            .map_err(IntegrationError::Database)?,
        rotated_at: row
            .try_get("rotated_at")
            .map_err(IntegrationError::Database)?,
        last_used_at: row
            .try_get("last_used_at")
            .map_err(IntegrationError::Database)?,
    })
}

fn report_summary(row: &sqlx::postgres::PgRow) -> Result<Value, IntegrationError> {
    Ok(
        json!({"schema_version":1,"id":row.try_get::<Uuid,_>("id").map_err(IntegrationError::Database)?,"title":row.try_get::<String,_>("name").map_err(IntegrationError::Database)?,"lifecycle_status":row.try_get::<String,_>("status").map_err(IntegrationError::Database)?,"review_open":row.try_get::<bool,_>("review_open").map_err(IntegrationError::Database)?,"programme_code":row.try_get::<Option<String>,_>("programme_code").map_err(IntegrationError::Database)?,"academic_year":row.try_get::<Option<String>,_>("academic_year").map_err(IntegrationError::Database)?,"semester":row.try_get::<Option<String>,_>("semester").map_err(IntegrationError::Database)?,"main_template":{"id":row.try_get::<Option<Uuid>,_>("template_id").map_err(IntegrationError::Database)?,"name":row.try_get::<Option<String>,_>("template_name").map_err(IntegrationError::Database)?},"front_matter_pack":{"id":row.try_get::<Option<Uuid>,_>("front_matter_pack_id").map_err(IntegrationError::Database)?,"name":row.try_get::<Option<String>,_>("front_matter_pack_name").map_err(IntegrationError::Database)?},"created_at":row.try_get::<String,_>("created_at").map_err(IntegrationError::Database)?,"updated_at":row.try_get::<String,_>("updated_at").map_err(IntegrationError::Database)?}),
    )
}

fn version_json(row: &sqlx::postgres::PgRow) -> Result<Value, IntegrationError> {
    let manifest: Value = row
        .try_get("manifest")
        .map_err(IntegrationError::Database)?;
    Ok(
        json!({"schema_version":1,"report_id":row.try_get::<Uuid,_>("paper_id").map_err(IntegrationError::Database)?,"id":row.try_get::<Uuid,_>("id").map_err(IntegrationError::Database)?,"document_epoch":row.try_get::<i64,_>("document_epoch").map_err(IntegrationError::Database)?,"version_number":row.try_get::<i64,_>("version_number").map_err(IntegrationError::Database)?,"version_type":row.try_get::<String,_>("version_type").map_err(IntegrationError::Database)?,"name":row.try_get::<Option<String>,_>("name").map_err(IntegrationError::Database)?,"workspace_version":row.try_get::<i64,_>("workspace_version").map_err(IntegrationError::Database)?,"snapshot_id":row.try_get::<String,_>("snapshot_id").map_err(IntegrationError::Database)?,"state_hash":row.try_get::<String,_>("state_hash").map_err(IntegrationError::Database)?,"files":manifest.get("workspace").and_then(|value|value.get("files")).cloned().unwrap_or_else(||json!({})),"file_identities":manifest.get("file_identities").cloned().unwrap_or_else(||json!([])),"file_policies":manifest.get("template_policy_provenance").and_then(|value|value.get("file_policies")).cloned().unwrap_or_else(||json!([])),"captured_front_matter":manifest.get("front_matter_resolved").cloned().unwrap_or_else(||json!({"status":"not_recorded"})),"captured_project_metadata":manifest.get("project_metadata").cloned().unwrap_or_else(||json!({"status":"not_recorded"})),"created_at":row.try_get::<String,_>("created_at").map_err(IntegrationError::Database)?}),
    )
}

pub fn parse_blob_hash(value: &str) -> Result<BlobHash, IntegrationError> {
    BlobHash::from_str(value).map_err(|_| IntegrationError::Integrity("invalid blob hash".into()))
}
