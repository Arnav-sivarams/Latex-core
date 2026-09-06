//! Final V2 governance: Team Leader reverts, file policies, and append-only restore cutovers.

use crate::{GlobalRole, V2Error, V2Repository};
use core_types::{
    BlobHash, LogicalPath, SnapshotId, TenantId, UserId, WorkspaceId, WorkspaceManifestV1,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};
use std::{collections::BTreeMap, str::FromStr};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct TeamTemplateResolutionInput {
    pub dominant_programme_code: Option<String>,
    pub resolution_method: String,
    pub external_team_key: Option<String>,
    pub source_import_job_id: Option<Uuid>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum V2FilePolicy {
    Editable,
    ContentReadOnly,
    StructureLocked,
    TemplateManaged,
    HiddenSystem,
}

impl V2FilePolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Editable => "EDITABLE",
            Self::ContentReadOnly => "CONTENT_READ_ONLY",
            Self::StructureLocked => "STRUCTURE_LOCKED",
            Self::TemplateManaged => "TEMPLATE_MANAGED",
            Self::HiddenSystem => "HIDDEN_SYSTEM",
        }
    }

    pub fn parse(value: &str) -> Result<Self, V2Error> {
        match value {
            "EDITABLE" => Ok(Self::Editable),
            "CONTENT_READ_ONLY" => Ok(Self::ContentReadOnly),
            "STRUCTURE_LOCKED" => Ok(Self::StructureLocked),
            "TEMPLATE_MANAGED" => Ok(Self::TemplateManaged),
            "HIDDEN_SYSTEM" => Ok(Self::HiddenSystem),
            _ => Err(V2Error::Integrity {
                message: format!("invalid V2 file policy {value}"),
            }),
        }
    }

    #[must_use]
    pub const fn content_editable(self) -> bool {
        matches!(self, Self::Editable | Self::StructureLocked)
    }

    #[must_use]
    pub const fn structure_editable(self) -> bool {
        matches!(self, Self::Editable)
    }

    #[must_use]
    pub const fn visible_to_participants(self) -> bool {
        !matches!(self, Self::HiddenSystem)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct V2FilePolicyRecord {
    pub file_id: Uuid,
    pub workspace_id: WorkspaceId,
    pub path: LogicalPath,
    pub policy: V2FilePolicy,
    pub updated_by_admin_user_id: Option<UserId>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RestorationRequest {
    pub id: Uuid,
    pub paper_id: Uuid,
    pub workspace_id: WorkspaceId,
    pub paper_name: String,
    pub requested_by_writer_user_id: UserId,
    pub writer_email: String,
    pub target_version_id: Uuid,
    pub target_version_number: u64,
    pub state: String,
    pub reason: Option<String>,
    pub mentor_user_id: Option<UserId>,
    pub mentor_decision_note: Option<String>,
    pub admin_user_id: Option<UserId>,
    pub admin_decision_note: Option<String>,
    pub leader_writer_user_id: Option<UserId>,
    pub leader_decision_note: Option<String>,
    pub created_at: String,
    pub submitted_at: Option<String>,
    pub mentor_decided_at: Option<String>,
    pub admin_decided_at: Option<String>,
    pub leader_decided_at: Option<String>,
    pub applied_version_id: Option<Uuid>,
}

#[derive(Clone, Debug)]
pub struct ExactRestoreState {
    pub document_epoch: u64,
    pub workspace_version: u64,
    pub snapshot_id: SnapshotId,
    pub manifest: Value,
    pub state_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RestorationApplied {
    pub applied_version_id: Uuid,
    pub safety_version_id: Uuid,
    pub document_epoch: u64,
    pub workspace_version: u64,
}

#[derive(Clone, Debug)]
pub struct TemplateSeedFile {
    pub path: LogicalPath,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
    pub policy: V2FilePolicy,
}

#[derive(Clone, Debug)]
pub struct TemplateChangeFile {
    pub path: LogicalPath,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
    pub policy: V2FilePolicy,
    pub existing_file_id: Option<Uuid>,
}

#[derive(Clone, Debug)]
pub struct TemplateChangeRequest {
    pub paper_id: Uuid,
    pub workspace_id: WorkspaceId,
    pub expected_workspace_version: u64,
    pub expected_template_id: Uuid,
    pub new_template_id: Uuid,
    pub new_source_identity: String,
    pub new_main_file: Option<LogicalPath>,
    pub files: Vec<TemplateChangeFile>,
    pub safety: ExactRestoreState,
}

impl V2Repository {
    pub async fn paper_template_pin(
        &self,
        admin: UserId,
        paper_id: Uuid,
    ) -> Result<Option<Value>, V2Error> {
        require_role_pool(self.database.pool(), admin, GlobalRole::Admin).await?;
        let row = sqlx::query(
            "SELECT p.paper_id,p.workspace_id,p.template_id,t.name AS template_name,p.source_identity,\
                    p.pinned_by_admin_user_id,p.pinned_at::text AS pinned_at \
             FROM latex_core.paper_template_pins p JOIN latex_core.templates t ON t.id=p.template_id WHERE p.paper_id=$1",
        ).bind(paper_id).fetch_optional(self.database.pool()).await.map_err(V2Error::Database)?;
        row.map(|row| Ok(json!({
            "paper_id":row.try_get::<Uuid,_>("paper_id").map_err(V2Error::Database)?,
            "workspace_id":row.try_get::<Uuid,_>("workspace_id").map_err(V2Error::Database)?,
            "template_id":row.try_get::<Uuid,_>("template_id").map_err(V2Error::Database)?,
            "template_name":row.try_get::<String,_>("template_name").map_err(V2Error::Database)?,
            "source_identity":row.try_get::<String,_>("source_identity").map_err(V2Error::Database)?,
            "pinned_by_admin_user_id":row.try_get::<Uuid,_>("pinned_by_admin_user_id").map_err(V2Error::Database)?,
            "pinned_at":row.try_get::<String,_>("pinned_at").map_err(V2Error::Database)?,
            "update_available":false,"update_status":"Template update unavailable in this RC"
        }))).transpose()
    }

    pub async fn admin_versions(&self, admin: UserId) -> Result<Vec<Value>, V2Error> {
        require_role_pool(self.database.pool(), admin, GlobalRole::Admin).await?;
        let rows = sqlx::query(
            "SELECT v.id,v.paper_id,t.name AS paper_name,v.version_number,v.version_type,v.name,c.email AS author,\
                    v.document_epoch,v.workspace_version,v.state_hash,v.created_at::text AS created_at \
             FROM latex_core.paper_versions v JOIN latex_core.paper_teams t ON t.id=v.paper_id \
             JOIN latex_core.user_credentials c ON c.user_id=v.created_by_user_id ORDER BY v.created_at DESC LIMIT 200",
        ).fetch_all(self.database.pool()).await.map_err(V2Error::Database)?;
        rows.into_iter().map(|row| Ok(json!({
            "id":row.try_get::<Uuid,_>("id").map_err(V2Error::Database)?,"paper_id":row.try_get::<Uuid,_>("paper_id").map_err(V2Error::Database)?,
            "paper_name":row.try_get::<String,_>("paper_name").map_err(V2Error::Database)?,"version_number":row.try_get::<i64,_>("version_number").map_err(V2Error::Database)?,
            "version_type":row.try_get::<String,_>("version_type").map_err(V2Error::Database)?,"name":row.try_get::<Option<String>,_>("name").map_err(V2Error::Database)?,
            "author":row.try_get::<String,_>("author").map_err(V2Error::Database)?,"document_epoch":row.try_get::<i64,_>("document_epoch").map_err(V2Error::Database)?,
            "workspace_version":row.try_get::<i64,_>("workspace_version").map_err(V2Error::Database)?,"state_hash":row.try_get::<String,_>("state_hash").map_err(V2Error::Database)?,
            "created_at":row.try_get::<String,_>("created_at").map_err(V2Error::Database)?
        }))).collect()
    }

    pub async fn admin_reviews(&self, admin: UserId) -> Result<Vec<Value>, V2Error> {
        require_role_pool(self.database.pool(), admin, GlobalRole::Admin).await?;
        let rows = sqlx::query(
            "SELECT rt.id,rr.id AS review_round_id,rr.paper_id,t.name AS paper_name,rt.thread_type,rt.severity,rt.category,rt.state,\
                    current_round.id AS current_review_round_id,current_round.status AS round_status,\
                    current_round.status='OPEN_FOR_REVIEW' AS review_open,\
                    mentor.email AS mentor,assigned.email AS assigned_writer,\
                    rt.created_at::text AS created_at,rt.updated_at::text AS updated_at \
             FROM latex_core.review_threads rt \
             JOIN latex_core.review_rounds rr ON rr.id=rt.review_round_id \
             JOIN latex_core.paper_teams t ON t.id=rr.paper_id \
             JOIN LATERAL (SELECT current_rr.id,current_rr.status FROM latex_core.review_rounds current_rr \
                           WHERE current_rr.paper_id=rr.paper_id ORDER BY current_rr.round_number DESC LIMIT 1) current_round ON true \
             JOIN latex_core.user_credentials mentor ON mentor.user_id=rt.created_by_mentor_user_id \
             LEFT JOIN latex_core.user_credentials assigned ON assigned.user_id=rt.assigned_writer_user_id \
             WHERE rt.publication_status='PUBLISHED' \
             ORDER BY rt.updated_at DESC LIMIT 200",
        ).fetch_all(self.database.pool()).await.map_err(V2Error::Database)?;
        rows.into_iter().map(|row| Ok(json!({
            "id":row.try_get::<Uuid,_>("id").map_err(V2Error::Database)?,"review_round_id":row.try_get::<Uuid,_>("review_round_id").map_err(V2Error::Database)?,
            "paper_id":row.try_get::<Uuid,_>("paper_id").map_err(V2Error::Database)?,"current_review_round_id":row.try_get::<Uuid,_>("current_review_round_id").map_err(V2Error::Database)?,
            "paper_name":row.try_get::<String,_>("paper_name").map_err(V2Error::Database)?,"thread_type":row.try_get::<String,_>("thread_type").map_err(V2Error::Database)?,
            "severity":row.try_get::<String,_>("severity").map_err(V2Error::Database)?,"category":row.try_get::<String,_>("category").map_err(V2Error::Database)?,
            "state":row.try_get::<String,_>("state").map_err(V2Error::Database)?,"round_status":row.try_get::<String,_>("round_status").map_err(V2Error::Database)?,
            "review_open":row.try_get::<bool,_>("review_open").map_err(V2Error::Database)?,
            "mentor":row.try_get::<String,_>("mentor").map_err(V2Error::Database)?,
            "assigned_writer":row.try_get::<Option<String>,_>("assigned_writer").map_err(V2Error::Database)?,
            "created_at":row.try_get::<String,_>("created_at").map_err(V2Error::Database)?,"updated_at":row.try_get::<String,_>("updated_at").map_err(V2Error::Database)?
        }))).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_template_paper_team(
        &self,
        admin: UserId,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        name: &str,
        leader_writer_id: UserId,
        writer_ids: &[UserId],
        mentor_ids: &[UserId],
        template_id: Uuid,
        source_identity: &str,
        main_path: &LogicalPath,
        files: &[TemplateSeedFile],
        resolution: Option<&TeamTemplateResolutionInput>,
    ) -> Result<(crate::PaperTeam, Vec<crate::PaperFile>), V2Error> {
        if name.trim().is_empty()
            || name.chars().count() > 200
            || files.is_empty()
            || !files.iter().any(|file| &file.path == main_path)
            || source_identity.len() != 64
        {
            return Err(V2Error::InvalidName);
        }
        if !writer_ids.contains(&leader_writer_id) {
            return Err(V2Error::Conflict {
                entity: "Paper Team Leader must be an assigned Writer",
            });
        }
        let mut all = writer_ids
            .iter()
            .chain(mentor_ids)
            .copied()
            .collect::<Vec<_>>();
        all.sort_unstable();
        all.dedup();
        if all.len() != writer_ids.len() + mentor_ids.len() {
            return Err(V2Error::Conflict {
                entity: "duplicate Paper Team assignment",
            });
        }
        all.push(admin);
        all.sort_unstable();
        all.dedup();
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_governance_users_tx(&mut tx, &all).await?;
        require_role_tx(&mut tx, admin, GlobalRole::Admin).await?;
        for writer in writer_ids {
            require_role_tx(&mut tx, *writer, GlobalRole::Writer).await?;
        }
        for mentor in mentor_ids {
            require_role_tx(&mut tx, *mentor, GlobalRole::Mentor).await?;
        }
        let template_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.templates WHERE id=$1)")
                .bind(template_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(V2Error::Database)?;
        if !template_exists {
            return Err(V2Error::NotFound { entity: "template" });
        }
        sqlx::query(
            "INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)",
        )
        .bind(workspace_id.as_uuid())
        .bind(tenant_id.as_uuid())
        .bind(admin.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        sqlx::query(
            "INSERT INTO latex_core.workspace_heads (workspace_id,durable_version) VALUES ($1,1)",
        )
        .bind(workspace_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        let mut operations = files.iter().map(|file| json!({"op":"put_file","path":file.path,"blob_hash":file.blob_hash,"size_bytes":file.size_bytes})).collect::<Vec<_>>();
        operations.push(json!({"op":"set_main_file","path":main_path}));
        sqlx::query("INSERT INTO latex_core.workspace_events (workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) VALUES ($1,1,$2,0,'workspace.mutation',1,$3,$4)")
            .bind(workspace_id.as_uuid()).bind(Uuid::new_v4()).bind(json!({"schema_version":1,"operations":operations})).bind(admin.as_uuid())
            .execute(&mut *tx).await.map_err(V2Error::Database)?;
        let team_row = sqlx::query("INSERT INTO latex_core.paper_teams (id,workspace_id,name,created_by_user_id) VALUES ($1,$2,$3,$4) RETURNING id,workspace_id,name,status,created_by_user_id,created_at::text AS created_at,updated_at::text AS updated_at")
            .bind(Uuid::new_v4()).bind(workspace_id.as_uuid()).bind(name.trim()).bind(admin.as_uuid())
            .fetch_one(&mut *tx).await.map_err(V2Error::Database)?;
        let team = crate::v2::decode_paper_team(team_row)?;
        for member in writer_ids.iter().chain(mentor_ids) {
            let writer_order = writer_ids
                .iter()
                .position(|writer| writer == member)
                .and_then(|position| i32::try_from(position + 1).ok());
            sqlx::query("INSERT INTO latex_core.paper_team_members (paper_team_id,user_id,assigned_by_user_id,is_leader,writer_order) VALUES ($1,$2,$3,$4,$5)")
                .bind(team.id).bind(member.as_uuid()).bind(admin.as_uuid()).bind(*member == leader_writer_id).bind(writer_order)
                .execute(&mut *tx).await.map_err(V2Error::Database)?;
        }
        let mut paper_files = Vec::with_capacity(files.len());
        for file in files {
            let file_id = Uuid::new_v4();
            let row = sqlx::query("INSERT INTO latex_core.paper_files (file_id,workspace_id,path) VALUES ($1,$2,$3) RETURNING file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at")
                .bind(file_id).bind(workspace_id.as_uuid()).bind(file.path.as_str()).fetch_one(&mut *tx).await.map_err(V2Error::Database)?;
            if file.policy != V2FilePolicy::Editable {
                sqlx::query("INSERT INTO latex_core.paper_file_policies (file_id,workspace_id,policy,updated_by_admin_user_id) VALUES ($1,$2,$3,$4)")
                    .bind(file_id).bind(workspace_id.as_uuid()).bind(file.policy.as_str()).bind(admin.as_uuid()).execute(&mut *tx).await.map_err(V2Error::Database)?;
            }
            paper_files.push(crate::v2::decode_paper_file(row)?);
        }
        sqlx::query("INSERT INTO latex_core.paper_template_pins (paper_id,workspace_id,template_id,source_identity,pinned_by_admin_user_id) VALUES ($1,$2,$3,$4,$5)")
            .bind(team.id).bind(workspace_id.as_uuid()).bind(template_id).bind(source_identity).bind(admin.as_uuid())
            .execute(&mut *tx).await.map_err(V2Error::Database)?;
        if let Some(resolution) = resolution {
            sqlx::query("INSERT INTO latex_core.paper_template_resolutions (paper_team_id,selected_template_id,dominant_programme_code,resolution_method,manual_override) VALUES ($1,$2,$3,$4,$5)")
                .bind(team.id).bind(template_id).bind(&resolution.dominant_programme_code).bind(&resolution.resolution_method).bind(resolution.resolution_method == "MANUAL_OVERRIDE")
                .execute(&mut *tx).await.map_err(V2Error::Database)?;
        }
        if let Some((external_team_key, source_import_job_id)) = resolution.and_then(|value| {
            value
                .external_team_key
                .as_deref()
                .zip(value.source_import_job_id)
        }) {
            sqlx::query("INSERT INTO latex_core.external_paper_team_links (external_team_key,paper_team_id,source_import_job_id) VALUES ($1,$2,$3)")
                .bind(external_team_key).bind(team.id).bind(source_import_job_id)
                .execute(&mut *tx).await.map_err(V2Error::Database)?;
            sqlx::query("INSERT INTO latex_core.audit_events (id,actor_user_id,event_type,resource_type,resource_id,metadata) VALUES ($1,$2,'institution.team.materialized','paper_team',$3,$4)")
                .bind(Uuid::new_v4()).bind(admin.as_uuid()).bind(team.id).bind(json!({"external_team_key":external_team_key,"job_id":source_import_job_id,"template_id":template_id,"resolution_method":resolution.map(|value| value.resolution_method.as_str())}))
                .execute(&mut *tx).await.map_err(V2Error::Database)?;
        } else {
            sqlx::query("INSERT INTO latex_core.audit_events (id,actor_user_id,event_type,resource_type,resource_id,metadata) VALUES ($1,$2,'institution.team.manual_created','paper_team',$3,$4)")
                .bind(Uuid::new_v4()).bind(admin.as_uuid()).bind(team.id)
                .bind(json!({"template_id":template_id,"resolution_method":resolution.map_or("MANUAL_OVERRIDE", |value| value.resolution_method.as_str()),"writer_count":writer_ids.len(),"mentor_count":mentor_ids.len()}))
                .execute(&mut *tx).await.map_err(V2Error::Database)?;
        }
        tx.commit().await.map_err(V2Error::Database)?;
        Ok((team, paper_files))
    }

    pub async fn visible_paper_files(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<crate::PaperFile>, V2Error> {
        let rows = sqlx::query(
            "SELECT f.file_id,f.workspace_id,f.path,f.revision,f.tombstoned,\
                    f.created_at::text AS created_at,f.updated_at::text AS updated_at,f.tombstoned_at::text AS tombstoned_at \
             FROM latex_core.paper_files f LEFT JOIN latex_core.paper_file_policies p ON p.file_id=f.file_id \
             WHERE f.workspace_id=$1 AND NOT f.tombstoned AND COALESCE(p.policy,'EDITABLE')<>'HIDDEN_SYSTEM' ORDER BY f.path",
        )
        .bind(workspace_id.as_uuid())
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(crate::v2::decode_paper_file).collect()
    }

    pub async fn file_policy(&self, file_id: Uuid) -> Result<V2FilePolicy, V2Error> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT p.policy FROM latex_core.paper_files f LEFT JOIN latex_core.paper_file_policies p ON p.file_id=f.file_id WHERE f.file_id=$1",
        )
        .bind(file_id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "paper file" })?;
        V2FilePolicy::parse(value.as_deref().unwrap_or("EDITABLE"))
    }

    pub async fn admin_file_policies(
        &self,
        admin: UserId,
        paper_id: Uuid,
    ) -> Result<Vec<V2FilePolicyRecord>, V2Error> {
        require_role_pool(self.database.pool(), admin, GlobalRole::Admin).await?;
        let rows = sqlx::query(
            "SELECT f.file_id,f.workspace_id,f.path,COALESCE(p.policy,'EDITABLE') AS policy,\
                    p.updated_by_admin_user_id,p.updated_at::text AS updated_at \
             FROM latex_core.paper_teams t JOIN latex_core.paper_files f ON f.workspace_id=t.workspace_id \
             LEFT JOIN latex_core.paper_file_policies p ON p.file_id=f.file_id \
             WHERE t.id=$1 AND NOT f.tombstoned ORDER BY f.path",
        )
        .bind(paper_id)
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(decode_policy).collect()
    }

    pub async fn set_file_policy(
        &self,
        admin: UserId,
        paper_id: Uuid,
        file_id: Uuid,
        policy: V2FilePolicy,
    ) -> Result<V2FilePolicyRecord, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        require_role_tx(&mut tx, admin, GlobalRole::Admin).await?;
        let file = sqlx::query(
            "SELECT f.workspace_id,f.path,COALESCE(p.policy,'EDITABLE') AS policy \
             FROM latex_core.paper_files f JOIN latex_core.paper_teams t ON t.workspace_id=f.workspace_id \
             LEFT JOIN latex_core.paper_file_policies p ON p.file_id=f.file_id \
             WHERE t.id=$1 AND f.file_id=$2 AND NOT f.tombstoned FOR UPDATE OF f",
        )
        .bind(paper_id)
        .bind(file_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "Paper Team file" })?;
        let workspace_id: Uuid = file.try_get("workspace_id").map_err(V2Error::Database)?;
        let path: String = file.try_get("path").map_err(V2Error::Database)?;
        let current_policy: String = file.try_get("policy").map_err(V2Error::Database)?;
        if path.starts_with(".latex-core/frontmatter/")
            && (current_policy != "HIDDEN_SYSTEM" || policy != V2FilePolicy::HiddenSystem)
        {
            return Err(V2Error::Conflict {
                entity: "managed Front Matter file policy",
            });
        }
        sqlx::query(
            "INSERT INTO latex_core.paper_file_policies (file_id,workspace_id,policy,updated_by_admin_user_id) \
             VALUES ($1,$2,$3,$4) ON CONFLICT (file_id) DO UPDATE SET policy=EXCLUDED.policy,\
             updated_by_admin_user_id=EXCLUDED.updated_by_admin_user_id,updated_at=statement_timestamp()",
        )
        .bind(file_id)
        .bind(workspace_id)
        .bind(policy.as_str())
        .bind(admin.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        let row = sqlx::query(
            "SELECT f.file_id,f.workspace_id,f.path,p.policy,p.updated_by_admin_user_id,p.updated_at::text AS updated_at \
             FROM latex_core.paper_files f JOIN latex_core.paper_file_policies p ON p.file_id=f.file_id WHERE f.file_id=$1",
        )
        .bind(file_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        decode_policy(row)
    }

    pub async fn apply_template_change(
        &self,
        admin: UserId,
        request: &TemplateChangeRequest,
    ) -> Result<u64, V2Error> {
        if request.files.is_empty()
            || request.new_source_identity.len() != 64
            || request.safety.workspace_version != request.expected_workspace_version
        {
            return Err(V2Error::Integrity {
                message: "invalid template-change request".into(),
            });
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        require_role_tx(&mut tx, admin, GlobalRole::Admin).await?;
        workspace_mutation_lock(&mut tx, request.workspace_id).await?;
        let status: String = sqlx::query_scalar(
            "SELECT status FROM latex_core.paper_teams WHERE id=$1 AND workspace_id=$2 FOR UPDATE",
        )
        .bind(request.paper_id)
        .bind(request.workspace_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound {
            entity: "Paper Team",
        })?;
        if status == "archived" {
            return Err(V2Error::Conflict {
                entity: "archived Paper Team template",
            });
        }
        let actual: i64 = sqlx::query_scalar(
            "SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1 FOR UPDATE",
        )
        .bind(request.workspace_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        if u64::try_from(actual).ok() != Some(request.expected_workspace_version) {
            return Err(V2Error::VersionConflict {
                expected: request.expected_workspace_version,
                actual: u64::try_from(actual).unwrap_or_default(),
            });
        }
        let pinned: Uuid = sqlx::query_scalar(
            "SELECT template_id FROM latex_core.paper_template_pins WHERE paper_id=$1 FOR UPDATE",
        )
        .bind(request.paper_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        if pinned != request.expected_template_id {
            return Err(V2Error::Conflict {
                entity: "Paper Team template pin changed after preview",
            });
        }
        let number: i64 = sqlx::query_scalar(
            "SELECT COALESCE(max(version_number),0)+1 FROM latex_core.paper_versions WHERE workspace_id=$1",
        )
        .bind(request.workspace_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        sqlx::query(
            "INSERT INTO latex_core.paper_versions (id,paper_id,workspace_id,document_epoch,version_number,version_type,name,created_by_user_id,workspace_version,snapshot_id,manifest,state_hash) VALUES ($1,$2,$3,$4,$5,'manual_checkpoint','PRE_TEMPLATE_CHANGE',$6,$7,$8,$9,$10)",
        )
        .bind(Uuid::new_v4())
        .bind(request.paper_id)
        .bind(request.workspace_id.as_uuid())
        .bind(to_i64(request.safety.document_epoch)?)
        .bind(number)
        .bind(admin.as_uuid())
        .bind(actual)
        .bind(request.safety.snapshot_id.to_hex())
        .bind(&request.safety.manifest)
        .bind(&request.safety.state_hash)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        let mut operations = request.files.iter().map(|file| json!({
            "op":"put_file","path":file.path,"blob_hash":file.blob_hash,"size_bytes":file.size_bytes
        })).collect::<Vec<_>>();
        if let Some(main) = &request.new_main_file {
            operations.push(json!({"op":"set_main_file","path":main}));
        }
        let next = request
            .expected_workspace_version
            .checked_add(1)
            .ok_or_else(|| V2Error::Integrity {
                message: "workspace version overflow".into(),
            })?;
        sqlx::query(
            "INSERT INTO latex_core.workspace_events (workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) VALUES ($1,$2,$3,$4,'workspace.mutation',1,$5,$6)",
        )
        .bind(request.workspace_id.as_uuid())
        .bind(to_i64(next)?)
        .bind(Uuid::new_v4())
        .bind(actual)
        .bind(json!({"schema_version":1,"operations":operations}))
        .bind(admin.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        sqlx::query("UPDATE latex_core.workspace_heads SET durable_version=$2,updated_at=statement_timestamp() WHERE workspace_id=$1")
            .bind(request.workspace_id.as_uuid()).bind(to_i64(next)?).execute(&mut *tx).await.map_err(V2Error::Database)?;
        for file in &request.files {
            let file_id = if let Some(file_id) = file.existing_file_id {
                sqlx::query("UPDATE latex_core.paper_files SET revision=revision+1,updated_at=statement_timestamp() WHERE file_id=$1 AND workspace_id=$2 AND NOT tombstoned")
                    .bind(file_id).bind(request.workspace_id.as_uuid()).execute(&mut *tx).await.map_err(V2Error::Database)?;
                file_id
            } else {
                let file_id = Uuid::new_v4();
                sqlx::query("INSERT INTO latex_core.paper_files (file_id,workspace_id,path) VALUES ($1,$2,$3)")
                    .bind(file_id).bind(request.workspace_id.as_uuid()).bind(file.path.as_str()).execute(&mut *tx).await.map_err(V2Error::Database)?;
                file_id
            };
            sqlx::query("INSERT INTO latex_core.paper_file_policies (file_id,workspace_id,policy,updated_by_admin_user_id) VALUES ($1,$2,$3,$4) ON CONFLICT(file_id) DO UPDATE SET policy=EXCLUDED.policy,updated_by_admin_user_id=EXCLUDED.updated_by_admin_user_id,updated_at=statement_timestamp()")
                .bind(file_id).bind(request.workspace_id.as_uuid()).bind(file.policy.as_str()).bind(admin.as_uuid()).execute(&mut *tx).await.map_err(V2Error::Database)?;
        }
        sqlx::query("UPDATE latex_core.paper_template_pins SET template_id=$2,source_identity=$3,pinned_by_admin_user_id=$4,pinned_at=statement_timestamp() WHERE paper_id=$1")
            .bind(request.paper_id).bind(request.new_template_id).bind(&request.new_source_identity).bind(admin.as_uuid()).execute(&mut *tx).await.map_err(V2Error::Database)?;
        sqlx::query("INSERT INTO latex_core.paper_template_resolutions (paper_team_id,selected_template_id,dominant_programme_code,resolution_method,manual_override) VALUES ($1,$2,NULL,'MANUAL_OVERRIDE',TRUE) ON CONFLICT(paper_team_id) DO UPDATE SET selected_template_id=EXCLUDED.selected_template_id,resolution_method='MANUAL_OVERRIDE',manual_override=TRUE,resolved_at=statement_timestamp()")
            .bind(request.paper_id).bind(request.new_template_id).execute(&mut *tx).await.map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(next)
    }

    pub async fn finalize_template_change(
        &self,
        admin: UserId,
        paper_id: Uuid,
        workspace_id: WorkspaceId,
        template_id: Uuid,
        exact: ExactRestoreState,
    ) -> Result<Uuid, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        require_role_tx(&mut tx, admin, GlobalRole::Admin).await?;
        workspace_mutation_lock(&mut tx, workspace_id).await?;
        let pinned: Option<Uuid> = sqlx::query_scalar(
            "SELECT template_id FROM latex_core.paper_template_pins WHERE paper_id=$1 FOR UPDATE",
        )
        .bind(paper_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        if pinned != Some(template_id) {
            return Err(V2Error::Conflict {
                entity: "Paper Team template finalization",
            });
        }
        let number: i64 = sqlx::query_scalar("SELECT COALESCE(max(version_number),0)+1 FROM latex_core.paper_versions WHERE workspace_id=$1")
            .bind(workspace_id.as_uuid()).fetch_one(&mut *tx).await.map_err(V2Error::Database)?;
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO latex_core.paper_versions (id,paper_id,workspace_id,document_epoch,version_number,version_type,name,created_by_user_id,workspace_version,snapshot_id,manifest,state_hash) VALUES ($1,$2,$3,$4,$5,'template_update','TEMPLATE_UPDATE',$6,$7,$8,$9,$10)")
            .bind(id).bind(paper_id).bind(workspace_id.as_uuid()).bind(to_i64(exact.document_epoch)?).bind(number).bind(admin.as_uuid())
            .bind(to_i64(exact.workspace_version)?).bind(exact.snapshot_id.to_hex()).bind(&exact.manifest).bind(&exact.state_hash)
            .execute(&mut *tx).await.map_err(V2Error::Database)?;
        sqlx::query("INSERT INTO latex_core.audit_events (id,actor_user_id,event_type,resource_type,resource_id,metadata) VALUES ($1,$2,'institution.team.template_applied','paper_team',$3,$4)")
            .bind(Uuid::new_v4()).bind(admin.as_uuid()).bind(paper_id).bind(json!({"template_id":template_id,"version_id":id,"workspace_version":exact.workspace_version,"resolution_method":"MANUAL_OVERRIDE"}))
            .execute(&mut *tx).await.map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(id)
    }

    pub async fn create_restoration_request(
        &self,
        writer: UserId,
        paper_id: Uuid,
        target_version_id: Uuid,
        reason: Option<&str>,
    ) -> Result<RestorationRequest, V2Error> {
        validate_note(reason)?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        require_role_tx(&mut tx, writer, GlobalRole::Writer).await?;
        let workspace_id: Uuid = sqlx::query_scalar(
            "SELECT t.workspace_id FROM latex_core.paper_teams t JOIN latex_core.paper_team_members m ON m.paper_team_id=t.id \
             WHERE t.id=$1 AND m.user_id=$2 AND t.status<>'archived'",
        )
        .bind(paper_id)
        .bind(writer.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "assigned Team Paper" })?;
        ensure_target_version(&mut tx, paper_id, workspace_id, target_version_id).await?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO latex_core.restoration_requests \
             (id,paper_id,workspace_id,requested_by_writer_user_id,target_version_id,state,reason,submitted_at) \
             VALUES ($1,$2,$3,$4,$5,'REQUESTED',$6,statement_timestamp())",
        )
        .bind(id).bind(paper_id).bind(workspace_id).bind(writer.as_uuid()).bind(target_version_id)
        .bind(reason.map(str::trim).filter(|value| !value.is_empty()))
        .execute(&mut *tx).await.map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        tracing::info!(
            request_id = %id,
            %paper_id,
            writer_user_id = %writer,
            %target_version_id,
            state = "REQUESTED",
            "Team revert requested"
        );
        self.restoration_request(id).await
    }

    pub async fn submit_restoration_request(
        &self,
        writer: UserId,
        request_id: Uuid,
    ) -> Result<RestorationRequest, V2Error> {
        require_role_pool(self.database.pool(), writer, GlobalRole::Writer).await?;
        let result = sqlx::query(
            "UPDATE latex_core.restoration_requests SET state='REQUESTED',submitted_at=statement_timestamp() \
             WHERE id=$1 AND requested_by_writer_user_id=$2 AND state='DRAFT'",
        )
        .bind(request_id).bind(writer.as_uuid()).execute(self.database.pool()).await.map_err(V2Error::Database)?;
        if result.rows_affected() != 1 {
            return Err(V2Error::Conflict {
                entity: "restoration request transition",
            });
        }
        self.restoration_request(request_id).await
    }

    pub async fn mentor_decide_restoration(
        &self,
        mentor: UserId,
        _request_id: Uuid,
        _endorse: bool,
        _note: Option<&str>,
    ) -> Result<RestorationRequest, V2Error> {
        require_role_pool(self.database.pool(), mentor, GlobalRole::Mentor).await?;
        Err(V2Error::Conflict {
            entity: "deprecated Mentor restoration workflow",
        })
    }

    pub async fn admin_reject_restoration(
        &self,
        admin: UserId,
        _request_id: Uuid,
        _note: Option<&str>,
    ) -> Result<RestorationRequest, V2Error> {
        require_role_pool(self.database.pool(), admin, GlobalRole::Admin).await?;
        Err(V2Error::Conflict {
            entity: "deprecated Admin restoration workflow",
        })
    }

    pub async fn leader_reject_restoration(
        &self,
        leader: UserId,
        request_id: Uuid,
        note: Option<&str>,
    ) -> Result<RestorationRequest, V2Error> {
        validate_note(note)?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        require_role_tx(&mut tx, leader, GlobalRole::Writer).await?;
        let paper_id: Uuid = sqlx::query_scalar(
            "SELECT paper_id FROM latex_core.restoration_requests \
             WHERE id=$1 AND state='REQUESTED' FOR UPDATE",
        )
        .bind(request_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::Conflict {
            entity: "requested Team revert",
        })?;
        require_team_leader_tx(&mut tx, leader, paper_id).await?;
        let result = sqlx::query(
            "UPDATE latex_core.restoration_requests \
             SET state='LEADER_REJECTED',leader_writer_user_id=$2,leader_decision_note=$3,leader_decided_at=statement_timestamp() \
             WHERE id=$1 AND state='REQUESTED'",
        )
        .bind(request_id).bind(leader.as_uuid()).bind(note.map(str::trim).filter(|value| !value.is_empty()))
        .execute(&mut *tx).await.map_err(V2Error::Database)?;
        if result.rows_affected() != 1 {
            return Err(V2Error::Conflict {
                entity: "requested Team revert",
            });
        }
        tx.commit().await.map_err(V2Error::Database)?;
        tracing::info!(
            %request_id,
            leader_writer_user_id = %leader,
            state = "LEADER_REJECTED",
            "Team revert request rejected"
        );
        self.restoration_request(request_id).await
    }

    pub async fn restoration_requests_for_actor(
        &self,
        actor: UserId,
        paper_id: Option<Uuid>,
    ) -> Result<Vec<RestorationRequest>, V2Error> {
        let role = role_pool(self.database.pool(), actor).await?;
        let rows = sqlx::query(&format!(
            "{} WHERE ($2::uuid IS NULL OR r.paper_id=$2) AND ({}) ORDER BY r.created_at DESC",
            restoration_select(),
            match role {
                GlobalRole::Admin => "TRUE",
                GlobalRole::Writer => "r.requested_by_writer_user_id=$1 OR EXISTS(SELECT 1 FROM latex_core.paper_team_members m WHERE m.paper_team_id=r.paper_id AND m.user_id=$1 AND m.is_leader)",
                GlobalRole::Mentor => "FALSE",
            }
        ))
        .bind(actor.as_uuid()).bind(paper_id)
        .fetch_all(self.database.pool()).await.map_err(V2Error::Database)?;
        rows.into_iter().map(decode_request).collect()
    }

    pub async fn apply_team_restoration(
        &self,
        leader: UserId,
        request_id: Uuid,
        safety: ExactRestoreState,
        note: Option<&str>,
    ) -> Result<RestorationApplied, V2Error> {
        validate_note(note)?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        require_role_tx(&mut tx, leader, GlobalRole::Writer).await?;
        let row = sqlx::query(
            "SELECT r.paper_id,r.workspace_id,r.target_version_id FROM latex_core.restoration_requests r \
             WHERE r.id=$1 AND r.state='REQUESTED' FOR UPDATE",
        )
        .bind(request_id).fetch_optional(&mut *tx).await.map_err(V2Error::Database)?
        .ok_or(V2Error::Conflict {
            entity: "requested Team revert",
        })?;
        let paper_id: Uuid = row.try_get("paper_id").map_err(V2Error::Database)?;
        require_team_leader_tx(&mut tx, leader, paper_id).await?;
        let workspace_id =
            WorkspaceId::from_uuid(row.try_get("workspace_id").map_err(V2Error::Database)?);
        let target_version_id: Uuid = row
            .try_get("target_version_id")
            .map_err(V2Error::Database)?;
        let applied = apply_restore(
            &mut tx,
            leader,
            paper_id,
            workspace_id,
            target_version_id,
            safety,
            RestoreKind::Team,
        )
        .await?;
        sqlx::query(
            "UPDATE latex_core.restoration_requests SET state='APPLIED',leader_writer_user_id=$2,leader_decision_note=$3,\
             leader_decided_at=statement_timestamp(),applied_version_id=$4 WHERE id=$1",
        )
        .bind(request_id).bind(leader.as_uuid()).bind(note.map(str::trim).filter(|value| !value.is_empty()))
        .bind(applied.applied_version_id).execute(&mut *tx).await.map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(applied)
    }

    pub async fn apply_direct_team_restoration(
        &self,
        leader: UserId,
        paper_id: Uuid,
        target_version_id: Uuid,
        safety: ExactRestoreState,
    ) -> Result<RestorationApplied, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        require_role_tx(&mut tx, leader, GlobalRole::Writer).await?;
        let workspace_id: Uuid = sqlx::query_scalar(
            "SELECT t.workspace_id FROM latex_core.paper_teams t \
             JOIN latex_core.paper_team_members m ON m.paper_team_id=t.id \
             WHERE t.id=$1 AND t.status<>'archived' AND m.user_id=$2 AND m.is_leader FOR UPDATE OF t",
        )
        .bind(paper_id)
        .bind(leader.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::RoleForbidden {
            user_id: leader,
            required: "Team Leader",
            actual: GlobalRole::Writer,
        })?;
        ensure_target_version(&mut tx, paper_id, workspace_id, target_version_id).await?;
        let applied = apply_restore(
            &mut tx,
            leader,
            paper_id,
            WorkspaceId::from_uuid(workspace_id),
            target_version_id,
            safety,
            RestoreKind::Team,
        )
        .await?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(applied)
    }

    pub async fn apply_personal_restoration(
        &self,
        writer: UserId,
        paper_id: Uuid,
        target_version_id: Uuid,
        safety: ExactRestoreState,
    ) -> Result<RestorationApplied, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        require_role_tx(&mut tx, writer, GlobalRole::Writer).await?;
        let workspace_id: Uuid = sqlx::query_scalar(
            "SELECT workspace_id FROM latex_core.personal_papers WHERE id=$1 AND owner_user_id=$2 AND status='active' FOR UPDATE",
        )
        .bind(paper_id).bind(writer.as_uuid()).fetch_optional(&mut *tx).await.map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "owned active personal paper" })?;
        let applied = apply_restore(
            &mut tx,
            writer,
            paper_id,
            WorkspaceId::from_uuid(workspace_id),
            target_version_id,
            safety,
            RestoreKind::Personal,
        )
        .await?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(applied)
    }

    async fn restoration_request(&self, id: Uuid) -> Result<RestorationRequest, V2Error> {
        let row = sqlx::query(&format!("{} WHERE r.id=$1", restoration_select()))
            .bind(id)
            .fetch_optional(self.database.pool())
            .await
            .map_err(V2Error::Database)?
            .ok_or(V2Error::NotFound {
                entity: "restoration request",
            })?;
        decode_request(row)
    }
}

pub(crate) async fn workspace_mutation_lock(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
) -> Result<(), V2Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text,0))")
        .bind(workspace_id.as_uuid().to_string())
        .execute(&mut **tx)
        .await
        .map_err(V2Error::Database)?;
    Ok(())
}

pub(crate) async fn assert_content_policy(
    tx: &mut Transaction<'_, Postgres>,
    file_id: Uuid,
) -> Result<(), V2Error> {
    let policy = policy_tx(tx, file_id).await?;
    if policy.content_editable() {
        Ok(())
    } else {
        Err(V2Error::Conflict {
            entity: "file content policy",
        })
    }
}

pub(crate) async fn assert_structure_policy(
    tx: &mut Transaction<'_, Postgres>,
    file_id: Uuid,
) -> Result<(), V2Error> {
    let policy = policy_tx(tx, file_id).await?;
    if policy.structure_editable() {
        Ok(())
    } else {
        Err(V2Error::Conflict {
            entity: "file structure policy",
        })
    }
}

pub(crate) async fn assert_main_policy(
    tx: &mut Transaction<'_, Postgres>,
    file_id: Uuid,
) -> Result<(), V2Error> {
    let policy = policy_tx(tx, file_id).await?;
    if matches!(
        policy,
        V2FilePolicy::Editable | V2FilePolicy::StructureLocked
    ) {
        Ok(())
    } else {
        Err(V2Error::Conflict {
            entity: "main-file policy",
        })
    }
}

async fn policy_tx(
    tx: &mut Transaction<'_, Postgres>,
    file_id: Uuid,
) -> Result<V2FilePolicy, V2Error> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT p.policy FROM latex_core.paper_files f LEFT JOIN latex_core.paper_file_policies p ON p.file_id=f.file_id WHERE f.file_id=$1",
    ).bind(file_id).fetch_optional(&mut **tx).await.map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "paper file" })?;
    V2FilePolicy::parse(value.as_deref().unwrap_or("EDITABLE"))
}

#[derive(Copy, Clone)]
enum RestoreKind {
    Team,
    Personal,
}

impl RestoreKind {
    const fn version_type(self) -> &'static str {
        match self {
            Self::Team => "team_revert",
            Self::Personal => "admin_restoration",
        }
    }

    const fn version_name(self) -> &'static str {
        match self {
            Self::Team => "Team revert",
            Self::Personal => "Personal restoration",
        }
    }
}

async fn apply_restore(
    tx: &mut Transaction<'_, Postgres>,
    actor: UserId,
    paper_id: Uuid,
    workspace_id: WorkspaceId,
    target_version_id: Uuid,
    safety: ExactRestoreState,
    kind: RestoreKind,
) -> Result<RestorationApplied, V2Error> {
    if safety.workspace_version == 0 || safety.state_hash.len() != 64 {
        return Err(V2Error::Integrity {
            message: "invalid pre-restore exact state".to_owned(),
        });
    }
    workspace_mutation_lock(tx, workspace_id).await?;
    let actual: i64 = sqlx::query_scalar(
        "SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1 FOR UPDATE",
    )
    .bind(workspace_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    if u64::try_from(actual).ok() != Some(safety.workspace_version) {
        return Err(V2Error::VersionConflict {
            expected: safety.workspace_version,
            actual: u64::try_from(actual).unwrap_or_default(),
        });
    }
    let target = sqlx::query(
        "SELECT document_epoch,workspace_version,snapshot_id,manifest,state_hash FROM latex_core.paper_versions \
         WHERE id=$1 AND paper_id=$2 AND workspace_id=$3",
    ).bind(target_version_id).bind(paper_id).bind(workspace_id.as_uuid())
        .fetch_optional(&mut **tx).await.map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "target paper version" })?;
    let target_manifest: Value = target.try_get("manifest").map_err(V2Error::Database)?;
    let workspace_manifest: WorkspaceManifestV1 =
        serde_json::from_value(target_manifest.get("workspace").cloned().ok_or_else(|| {
            V2Error::Integrity {
                message: "target version lacks workspace manifest".to_owned(),
            }
        })?)
        .map_err(|error| V2Error::Integrity {
            message: format!("invalid target workspace manifest: {error}"),
        })?;
    let target_snapshot: String = target.try_get("snapshot_id").map_err(V2Error::Database)?;
    let target_hash: String = target.try_get("state_hash").map_err(V2Error::Database)?;
    let number: i64 = sqlx::query_scalar(
        "SELECT COALESCE(max(version_number),0)+1 FROM latex_core.paper_versions WHERE workspace_id=$1",
    ).bind(workspace_id.as_uuid()).fetch_one(&mut **tx).await.map_err(V2Error::Database)?;
    let safety_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO latex_core.paper_versions \
         (id,paper_id,workspace_id,document_epoch,version_number,version_type,name,created_by_user_id,workspace_version,snapshot_id,manifest,state_hash) \
         VALUES ($1,$2,$3,$4,$5,'pre_restore_safety','Pre-restore safety',$6,$7,$8,$9,$10)",
    ).bind(safety_id).bind(paper_id).bind(workspace_id.as_uuid()).bind(to_i64(safety.document_epoch)?)
        .bind(number).bind(actor.as_uuid()).bind(actual).bind(safety.snapshot_id.to_hex()).bind(&safety.manifest).bind(&safety.state_hash)
        .execute(&mut **tx).await.map_err(V2Error::Database)?;

    if matches!(kind, RestoreKind::Team) {
        restore_front_matter_version_state(
            tx,
            actor,
            paper_id,
            target_manifest.get("front_matter"),
        )
        .await?;
    }

    let rows = sqlx::query(
        "SELECT file_id,path,tombstoned FROM latex_core.paper_files WHERE workspace_id=$1 FOR UPDATE",
    ).bind(workspace_id.as_uuid()).fetch_all(&mut **tx).await.map_err(V2Error::Database)?;
    let mut by_id = BTreeMap::new();
    let mut by_live_path = BTreeMap::new();
    for row in rows {
        let id: Uuid = row.try_get("file_id").map_err(V2Error::Database)?;
        let path: String = row.try_get("path").map_err(V2Error::Database)?;
        let tombstoned: bool = row.try_get("tombstoned").map_err(V2Error::Database)?;
        by_id.insert(id, path.clone());
        if !tombstoned {
            by_live_path.insert(path, id);
        }
    }
    let identities = target_manifest
        .get("file_identities")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                entry.get("path")?.as_str()?.to_owned(),
                Uuid::parse_str(entry.get("file_id")?.as_str()?).ok()?,
            ))
        })
        .collect::<BTreeMap<_, _>>();
    sqlx::query(
        "UPDATE latex_core.paper_files SET tombstoned=TRUE,tombstoned_at=statement_timestamp(),revision=revision+1,updated_at=statement_timestamp() \
         WHERE workspace_id=$1 AND NOT tombstoned",
    ).bind(workspace_id.as_uuid()).execute(&mut **tx).await.map_err(V2Error::Database)?;
    let mut operations = by_live_path
        .keys()
        .map(|path| json!({"op":"delete_file","path":path}))
        .collect::<Vec<_>>();
    for (path, entry) in workspace_manifest.files() {
        let path_text = path.as_str().to_owned();
        let chosen = identities
            .get(&path_text)
            .copied()
            .or_else(|| by_live_path.get(&path_text).copied())
            .unwrap_or_else(Uuid::new_v4);
        if let Some(existing_path) = by_id.get(&chosen) {
            if identities.get(&path_text) == Some(&chosen) && existing_path != &path_text {
                // Historical identity is authoritative; restoration may legitimately restore its old path.
            }
            sqlx::query(
                "UPDATE latex_core.paper_files SET path=$2,tombstoned=FALSE,tombstoned_at=NULL,revision=revision+1,updated_at=statement_timestamp() \
                 WHERE file_id=$1 AND workspace_id=$3",
            ).bind(chosen).bind(&path_text).bind(workspace_id.as_uuid()).execute(&mut **tx).await.map_err(V2Error::Database)?;
        } else {
            sqlx::query(
                "INSERT INTO latex_core.paper_files (file_id,workspace_id,path) VALUES ($1,$2,$3)",
            )
            .bind(chosen)
            .bind(workspace_id.as_uuid())
            .bind(&path_text)
            .execute(&mut **tx)
            .await
            .map_err(V2Error::Database)?;
        }
        operations.push(json!({"op":"put_file","path":path,"blob_hash":entry.blob_hash,"size_bytes":entry.size_bytes}));
    }
    operations.push(json!({"op":"set_main_file","path":workspace_manifest.main_file()}));
    let next = actual.checked_add(1).ok_or_else(|| V2Error::Integrity {
        message: "workspace version overflow".to_owned(),
    })?;
    sqlx::query(
        "INSERT INTO latex_core.workspace_events (workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) \
         VALUES ($1,$2,$3,$4,'workspace.mutation',1,$5,$6)",
    ).bind(workspace_id.as_uuid()).bind(next).bind(Uuid::new_v4()).bind(actual)
        .bind(json!({"schema_version":1,"operations":operations})).bind(actor.as_uuid())
        .execute(&mut **tx).await.map_err(V2Error::Database)?;
    sqlx::query(
        "INSERT INTO latex_core.workspace_snapshots (workspace_id,workspace_version,snapshot_id) VALUES ($1,$2,$3)",
    ).bind(workspace_id.as_uuid()).bind(next).bind(&target_snapshot).execute(&mut **tx).await.map_err(V2Error::Database)?;
    sqlx::query(
        "UPDATE latex_core.workspace_heads SET durable_version=$2,latest_snapshot_version=$2,updated_at=statement_timestamp() WHERE workspace_id=$1",
    ).bind(workspace_id.as_uuid()).bind(next).execute(&mut **tx).await.map_err(V2Error::Database)?;
    let epoch: i64 = sqlx::query_scalar(
        "UPDATE latex_core.paper_collaboration_state SET document_epoch=document_epoch+1,updated_at=statement_timestamp() WHERE workspace_id=$1 RETURNING document_epoch",
    ).bind(workspace_id.as_uuid()).fetch_one(&mut **tx).await.map_err(V2Error::Database)?;
    let mut applied_manifest = target_manifest;
    if let Some(object) = applied_manifest.as_object_mut() {
        object.insert("document_epoch".to_owned(), json!(epoch));
        object.insert("source_sequence".to_owned(), json!(next));
        object.insert(
            "restored_from_version_id".to_owned(),
            json!(target_version_id),
        );
    }
    let applied_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO latex_core.paper_versions \
         (id,paper_id,workspace_id,document_epoch,version_number,version_type,name,created_by_user_id,workspace_version,snapshot_id,manifest,state_hash) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    ).bind(applied_id).bind(paper_id).bind(workspace_id.as_uuid()).bind(epoch).bind(number + 1)
        .bind(kind.version_type()).bind(kind.version_name()).bind(actor.as_uuid()).bind(next).bind(&target_snapshot)
        .bind(&applied_manifest).bind(&target_hash)
        .execute(&mut **tx).await.map_err(V2Error::Database)?;
    sqlx::query(
        "UPDATE latex_core.v2_paper_build_state SET desired_state_hash=$2,desired_source_sequence=$3,active_build_id=NULL,\
         pending_snapshot_id=NULL,pending_manifest=NULL,pending_state_hash=NULL,pending_source_sequence=NULL,pending_document_epoch=NULL,\
         pending_tenant_id=NULL,pending_user_id=NULL,pending_trigger_type=NULL,pending_compile_key=NULL,pending_engine=NULL,\
         pending_tex_environment_id=NULL,pending_latexmk_profile=NULL,pending_shell_policy=NULL,pending_synctex=NULL,updated_at=statement_timestamp() WHERE workspace_id=$1",
    ).bind(workspace_id.as_uuid()).bind(&target_hash).bind(next).execute(&mut **tx).await.map_err(V2Error::Database)?;
    tracing::info!(%paper_id, %workspace_id, actor_user_id=%actor, target_version_id=%target_version_id, safety_version_id=%safety_id, applied_version_id=%applied_id, document_epoch=epoch, workspace_version=next, "governed restoration materialized as new workspace head");
    Ok(RestorationApplied {
        applied_version_id: applied_id,
        safety_version_id: safety_id,
        document_epoch: u64::try_from(epoch).map_err(|_| V2Error::Integrity {
            message: "negative restored epoch".to_owned(),
        })?,
        workspace_version: u64::try_from(next).map_err(|_| V2Error::Integrity {
            message: "negative restored workspace version".to_owned(),
        })?,
    })
}

async fn restore_front_matter_version_state(
    tx: &mut Transaction<'_, Postgres>,
    actor: UserId,
    paper_id: Uuid,
    state: Option<&Value>,
) -> Result<(), V2Error> {
    sqlx::query("DELETE FROM latex_core.paper_front_matter_sections WHERE paper_team_id=$1")
        .bind(paper_id)
        .execute(&mut **tx)
        .await
        .map_err(V2Error::Database)?;
    sqlx::query("DELETE FROM latex_core.paper_front_matter_values WHERE paper_team_id=$1")
        .bind(paper_id)
        .execute(&mut **tx)
        .await
        .map_err(V2Error::Database)?;
    sqlx::query("DELETE FROM latex_core.paper_front_matter_pins WHERE paper_team_id=$1")
        .bind(paper_id)
        .execute(&mut **tx)
        .await
        .map_err(V2Error::Database)?;
    let Some(state) = state.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    if state.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err(V2Error::Integrity {
            message: "invalid Front Matter version state".into(),
        });
    }
    let pin = state.get("pin").ok_or_else(|| V2Error::Integrity {
        message: "Front Matter version state has no pin".into(),
    })?;
    let pack_id = json_uuid(pin, "front_matter_pack_id")?;
    let dominant = pin.get("dominant_programme_code").and_then(Value::as_str);
    let method = pin
        .get("resolution_method")
        .and_then(Value::as_str)
        .ok_or_else(|| V2Error::Integrity {
            message: "Front Matter version state has no resolution method".into(),
        })?;
    let status = pin.get("status").and_then(Value::as_str).unwrap_or("READY");
    let missing = pin
        .get("missing_required_fields")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let last_error = pin.get("last_error").and_then(Value::as_str);
    sqlx::query("INSERT INTO latex_core.paper_front_matter_pins (paper_team_id,front_matter_pack_id,dominant_programme_code,resolution_method,assigned_by_user_id,status,missing_required_fields,last_error) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(paper_id).bind(pack_id).bind(dominant).bind(method).bind(actor.as_uuid()).bind(status).bind(missing).bind(last_error)
        .execute(&mut **tx).await.map_err(V2Error::Database)?;
    for value in state
        .get("values")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = value
            .get("field_key")
            .and_then(Value::as_str)
            .ok_or_else(|| V2Error::Integrity {
                message: "Front Matter version value has no key".into(),
            })?;
        let content = value
            .get("value_json")
            .cloned()
            .ok_or_else(|| V2Error::Integrity {
                message: "Front Matter version value has no value".into(),
            })?;
        let source = value
            .get("value_source")
            .and_then(Value::as_str)
            .ok_or_else(|| V2Error::Integrity {
                message: "Front Matter version value has no source".into(),
            })?;
        sqlx::query("INSERT INTO latex_core.paper_front_matter_values(paper_team_id,field_key,value_json,value_source,updated_by_user_id) VALUES($1,$2,$3,$4,$5)").bind(paper_id).bind(key).bind(content).bind(source).bind(actor.as_uuid()).execute(&mut **tx).await.map_err(V2Error::Database)?;
    }
    for section in state
        .get("sections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = section
            .get("section_key")
            .and_then(Value::as_str)
            .ok_or_else(|| V2Error::Integrity {
                message: "Front Matter version section has no key".into(),
            })?;
        let enabled = section
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| V2Error::Integrity {
                message: "Front Matter version section has no enabled state".into(),
            })?;
        sqlx::query("INSERT INTO latex_core.paper_front_matter_sections(paper_team_id,section_key,enabled,updated_by_user_id) VALUES($1,$2,$3,$4)").bind(paper_id).bind(key).bind(enabled).bind(actor.as_uuid()).execute(&mut **tx).await.map_err(V2Error::Database)?;
    }
    Ok(())
}

fn json_uuid(value: &Value, key: &str) -> Result<Uuid, V2Error> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| V2Error::Integrity {
            message: format!("Front Matter version state has invalid {key}"),
        })
}

async fn ensure_target_version(
    tx: &mut Transaction<'_, Postgres>,
    paper_id: Uuid,
    workspace_id: Uuid,
    target: Uuid,
) -> Result<(), V2Error> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM latex_core.paper_versions WHERE id=$1 AND paper_id=$2 AND workspace_id=$3)",
    ).bind(target).bind(paper_id).bind(workspace_id).fetch_one(&mut **tx).await.map_err(V2Error::Database)?;
    if exists {
        Ok(())
    } else {
        Err(V2Error::NotFound {
            entity: "target paper version",
        })
    }
}

fn restoration_select() -> &'static str {
    "SELECT r.*,t.name AS paper_name,c.email AS writer_email,v.version_number,r.created_at::text AS created_at_text,\
     r.submitted_at::text AS submitted_at_text,r.mentor_decided_at::text AS mentor_decided_at_text,\
     r.admin_decided_at::text AS admin_decided_at_text,r.leader_decided_at::text AS leader_decided_at_text \
     FROM latex_core.restoration_requests r \
     JOIN latex_core.paper_teams t ON t.id=r.paper_id JOIN latex_core.user_credentials c ON c.user_id=r.requested_by_writer_user_id \
     JOIN latex_core.paper_versions v ON v.id=r.target_version_id"
}

fn decode_request(row: PgRow) -> Result<RestorationRequest, V2Error> {
    let number: i64 = row.try_get("version_number").map_err(V2Error::Database)?;
    Ok(RestorationRequest {
        id: row.try_get("id").map_err(V2Error::Database)?,
        paper_id: row.try_get("paper_id").map_err(V2Error::Database)?,
        workspace_id: WorkspaceId::from_uuid(
            row.try_get("workspace_id").map_err(V2Error::Database)?,
        ),
        paper_name: row.try_get("paper_name").map_err(V2Error::Database)?,
        requested_by_writer_user_id: UserId::from_uuid(
            row.try_get("requested_by_writer_user_id")
                .map_err(V2Error::Database)?,
        ),
        writer_email: row.try_get("writer_email").map_err(V2Error::Database)?,
        target_version_id: row
            .try_get("target_version_id")
            .map_err(V2Error::Database)?,
        target_version_number: u64::try_from(number).map_err(|_| V2Error::Integrity {
            message: "negative target version number".to_owned(),
        })?,
        state: row.try_get("state").map_err(V2Error::Database)?,
        reason: row.try_get("reason").map_err(V2Error::Database)?,
        mentor_user_id: row
            .try_get::<Option<Uuid>, _>("mentor_user_id")
            .map_err(V2Error::Database)?
            .map(UserId::from_uuid),
        mentor_decision_note: row
            .try_get("mentor_decision_note")
            .map_err(V2Error::Database)?,
        admin_user_id: row
            .try_get::<Option<Uuid>, _>("admin_user_id")
            .map_err(V2Error::Database)?
            .map(UserId::from_uuid),
        admin_decision_note: row
            .try_get("admin_decision_note")
            .map_err(V2Error::Database)?,
        leader_writer_user_id: row
            .try_get::<Option<Uuid>, _>("leader_writer_user_id")
            .map_err(V2Error::Database)?
            .map(UserId::from_uuid),
        leader_decision_note: row
            .try_get("leader_decision_note")
            .map_err(V2Error::Database)?,
        created_at: row.try_get("created_at_text").map_err(V2Error::Database)?,
        submitted_at: row
            .try_get("submitted_at_text")
            .map_err(V2Error::Database)?,
        mentor_decided_at: row
            .try_get("mentor_decided_at_text")
            .map_err(V2Error::Database)?,
        admin_decided_at: row
            .try_get("admin_decided_at_text")
            .map_err(V2Error::Database)?,
        leader_decided_at: row
            .try_get("leader_decided_at_text")
            .map_err(V2Error::Database)?,
        applied_version_id: row
            .try_get("applied_version_id")
            .map_err(V2Error::Database)?,
    })
}

fn decode_policy(row: PgRow) -> Result<V2FilePolicyRecord, V2Error> {
    Ok(V2FilePolicyRecord {
        file_id: row.try_get("file_id").map_err(V2Error::Database)?,
        workspace_id: WorkspaceId::from_uuid(
            row.try_get("workspace_id").map_err(V2Error::Database)?,
        ),
        path: LogicalPath::parse(
            &row.try_get::<String, _>("path")
                .map_err(V2Error::Database)?,
        )
        .map_err(|error| V2Error::InvalidPath {
            message: error.to_string(),
        })?,
        policy: V2FilePolicy::parse(
            &row.try_get::<String, _>("policy")
                .map_err(V2Error::Database)?,
        )?,
        updated_by_admin_user_id: row
            .try_get::<Option<Uuid>, _>("updated_by_admin_user_id")
            .map_err(V2Error::Database)?
            .map(UserId::from_uuid),
        updated_at: row.try_get("updated_at").map_err(V2Error::Database)?,
    })
}

async fn role_pool(pool: &sqlx::PgPool, user: UserId) -> Result<GlobalRole, V2Error> {
    let value: String =
        sqlx::query_scalar("SELECT role FROM latex_core.global_user_roles WHERE user_id=$1")
            .bind(user.as_uuid())
            .fetch_optional(pool)
            .await
            .map_err(V2Error::Database)?
            .ok_or(V2Error::RoleMissing { user_id: user })?;
    GlobalRole::from_str(&value)
}

async fn require_role_pool(
    pool: &sqlx::PgPool,
    user: UserId,
    wanted: GlobalRole,
) -> Result<(), V2Error> {
    let actual = role_pool(pool, user).await?;
    if actual == wanted {
        Ok(())
    } else {
        Err(V2Error::RoleForbidden {
            user_id: user,
            required: wanted.as_str(),
            actual,
        })
    }
}

async fn require_role_tx(
    tx: &mut Transaction<'_, Postgres>,
    user: UserId,
    wanted: GlobalRole,
) -> Result<(), V2Error> {
    let value: String =
        sqlx::query_scalar("SELECT role FROM latex_core.global_user_roles WHERE user_id=$1")
            .bind(user.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(V2Error::Database)?
            .ok_or(V2Error::RoleMissing { user_id: user })?;
    let actual = GlobalRole::from_str(&value)?;
    if actual == wanted {
        Ok(())
    } else {
        Err(V2Error::RoleForbidden {
            user_id: user,
            required: wanted.as_str(),
            actual,
        })
    }
}

async fn lock_governance_users_tx(
    tx: &mut Transaction<'_, Postgres>,
    users: &[UserId],
) -> Result<(), V2Error> {
    for user in users {
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM latex_core.users WHERE id=$1 FOR UPDATE")
            .bind(user.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(V2Error::Database)?
            .ok_or(V2Error::NotFound { entity: "user" })?;
    }
    Ok(())
}

async fn require_team_leader_tx(
    tx: &mut Transaction<'_, Postgres>,
    writer: UserId,
    paper_id: Uuid,
) -> Result<(), V2Error> {
    let authorized: Option<Uuid> = sqlx::query_scalar(
        "SELECT t.id FROM latex_core.paper_teams t \
         JOIN latex_core.paper_team_members m ON m.paper_team_id=t.id \
         WHERE t.id=$1 AND t.status<>'archived' AND m.user_id=$2 AND m.is_leader FOR UPDATE OF t",
    )
    .bind(paper_id)
    .bind(writer.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    if authorized.is_some() {
        Ok(())
    } else {
        Err(V2Error::RoleForbidden {
            user_id: writer,
            required: "Team Leader",
            actual: GlobalRole::Writer,
        })
    }
}

fn validate_note(note: Option<&str>) -> Result<(), V2Error> {
    if note.is_some_and(|value| value.chars().count() > 4000) {
        Err(V2Error::Integrity {
            message: "governance note exceeds 4000 characters".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn to_i64(value: u64) -> Result<i64, V2Error> {
    i64::try_from(value).map_err(|_| V2Error::Integrity {
        message: "governance number exceeds PostgreSQL BIGINT".to_owned(),
    })
}
