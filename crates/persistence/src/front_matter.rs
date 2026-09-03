//! `PostgreSQL` persistence for immutable Front Matter Packs and atomic managed renders.

use crate::Database;
use core_types::{BlobHash, TenantId, UserId, WorkspaceId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use std::{collections::BTreeMap, str::FromStr};
use thiserror::Error;
use uuid::Uuid;

const MANAGED_PREFIX: &str = ".latex-core/frontmatter/";

#[derive(Debug, Error)]
pub enum FrontMatterRepositoryError {
    #[error("record not found")]
    NotFound,
    #[error("operation is not authorized")]
    Forbidden,
    #[error("Front Matter Pack is in use")]
    InUse,
    #[error("This template is not configured for Front Matter.")]
    IncompatibleTemplate,
    #[error("workspace changed during Front Matter rendering")]
    VersionConflict,
    #[error("Front Matter persistence failed")]
    Database(#[source] sqlx::Error),
    #[error("persistent Front Matter data is invalid: {0}")]
    Integrity(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrontMatterPackRecord {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub manifest_json: Value,
    pub content_hash: String,
    pub created_by_user_id: UserId,
    pub created_at: String,
    pub archived_at: Option<String>,
    pub usage_count: i64,
}

#[derive(Clone, Debug)]
pub struct FrontMatterPackFileRecord {
    pub path: String,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
    pub media_type: String,
}

#[derive(Clone, Debug)]
pub struct ManagedFrontMatterFile {
    pub path: String,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct FrontMatterValueRecord {
    pub field_key: String,
    pub value_json: Value,
    pub value_source: String,
}

#[derive(Clone, Debug)]
pub struct ExactStateRecord {
    pub document_epoch: u64,
    pub workspace_version: u64,
    pub snapshot_id: String,
    pub manifest: Value,
    pub state_hash: String,
}

#[derive(Clone, Debug)]
pub struct ApplyFrontMatterRequest {
    pub paper_team_id: Uuid,
    pub workspace_id: WorkspaceId,
    pub expected_workspace_version: u64,
    pub pack_id: Uuid,
    pub dominant_programme_code: Option<String>,
    pub resolution_method: String,
    pub files: Vec<ManagedFrontMatterFile>,
    pub values: Vec<FrontMatterValueRecord>,
    pub sections: BTreeMap<String, bool>,
    pub safety: Option<ExactStateRecord>,
}

#[derive(Clone, Debug)]
pub struct FrontMatterRepository {
    database: Database,
}

impl FrontMatterRepository {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "pack identity, metadata, immutable manifest, and file records are committed together"
    )]
    pub async fn create_pack(
        &self,
        actor: UserId,
        id: Uuid,
        name: &str,
        description: Option<&str>,
        manifest: &Value,
        content_hash: &str,
        files: &[FrontMatterPackFileRecord],
    ) -> Result<(), FrontMatterRepositoryError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        require_admin(&mut tx, actor).await?;
        sqlx::query("INSERT INTO latex_core.front_matter_packs (id,name,description,manifest_json,content_hash,created_by_user_id) VALUES ($1,$2,$3,$4,$5,$6)")
            .bind(id).bind(name).bind(description).bind(manifest).bind(content_hash).bind(actor.as_uuid())
            .execute(&mut *tx).await.map_err(map_conflict)?;
        for file in files {
            let size = i64::try_from(file.size_bytes).map_err(|_| {
                FrontMatterRepositoryError::Integrity("file size exceeds PostgreSQL BIGINT".into())
            })?;
            sqlx::query("INSERT INTO latex_core.front_matter_pack_files (pack_id,path,blob_hash,size_bytes,media_type) VALUES ($1,$2,$3,$4,$5)")
                .bind(id).bind(&file.path).bind(file.blob_hash.to_string()).bind(size).bind(&file.media_type)
                .execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        }
        audit(
            &mut tx,
            actor,
            "front_matter.pack.imported",
            "front_matter_pack",
            id,
            json!({"name":name,"content_hash":content_hash,"file_count":files.len()}),
        )
        .await?;
        tx.commit()
            .await
            .map_err(FrontMatterRepositoryError::Database)
    }

    pub async fn list_packs(
        &self,
    ) -> Result<Vec<FrontMatterPackRecord>, FrontMatterRepositoryError> {
        let rows = sqlx::query("SELECT p.id,p.name,p.description,p.manifest_json,p.content_hash,p.created_by_user_id,p.created_at::text,p.archived_at::text,count(pin.paper_team_id) AS usage_count FROM latex_core.front_matter_packs p LEFT JOIN latex_core.paper_front_matter_pins pin ON pin.front_matter_pack_id=p.id WHERE p.archived_at IS NULL GROUP BY p.id ORDER BY p.name")
            .fetch_all(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?;
        rows.into_iter().map(decode_pack).collect()
    }

    pub async fn pack(
        &self,
        id: Uuid,
    ) -> Result<FrontMatterPackRecord, FrontMatterRepositoryError> {
        let row = sqlx::query("SELECT p.id,p.name,p.description,p.manifest_json,p.content_hash,p.created_by_user_id,p.created_at::text,p.archived_at::text,(SELECT count(*) FROM latex_core.paper_front_matter_pins pin WHERE pin.front_matter_pack_id=p.id) AS usage_count FROM latex_core.front_matter_packs p WHERE p.id=$1")
            .bind(id).fetch_optional(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?.ok_or(FrontMatterRepositoryError::NotFound)?;
        decode_pack(row)
    }

    pub async fn pack_files(
        &self,
        id: Uuid,
    ) -> Result<Vec<FrontMatterPackFileRecord>, FrontMatterRepositoryError> {
        let rows = sqlx::query("SELECT path,blob_hash,size_bytes,media_type FROM latex_core.front_matter_pack_files WHERE pack_id=$1 ORDER BY path")
            .bind(id).fetch_all(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?;
        rows.into_iter()
            .map(|row| {
                let size: i64 = row
                    .try_get("size_bytes")
                    .map_err(FrontMatterRepositoryError::Database)?;
                Ok(FrontMatterPackFileRecord {
                    path: row
                        .try_get("path")
                        .map_err(FrontMatterRepositoryError::Database)?,
                    blob_hash: BlobHash::from_str(
                        &row.try_get::<String, _>("blob_hash")
                            .map_err(FrontMatterRepositoryError::Database)?,
                    )
                    .map_err(|error| FrontMatterRepositoryError::Integrity(error.to_string()))?,
                    size_bytes: u64::try_from(size).map_err(|_| {
                        FrontMatterRepositoryError::Integrity("negative file size".into())
                    })?,
                    media_type: row
                        .try_get("media_type")
                        .map_err(FrontMatterRepositoryError::Database)?,
                })
            })
            .collect()
    }

    pub async fn update_pack_metadata(
        &self,
        actor: UserId,
        id: Uuid,
        name: &str,
        description: Option<&str>,
    ) -> Result<(), FrontMatterRepositoryError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        require_admin(&mut tx, actor).await?;
        let changed = sqlx::query("UPDATE latex_core.front_matter_packs SET name=$2,description=$3 WHERE id=$1 AND archived_at IS NULL")
            .bind(id).bind(name).bind(description).execute(&mut *tx).await.map_err(map_conflict)?.rows_affected();
        if changed == 0 {
            return Err(FrontMatterRepositoryError::NotFound);
        }
        audit(
            &mut tx,
            actor,
            "front_matter.pack.metadata_updated",
            "front_matter_pack",
            id,
            json!({"name":name}),
        )
        .await?;
        tx.commit()
            .await
            .map_err(FrontMatterRepositoryError::Database)
    }

    pub async fn delete_pack_if_unused(
        &self,
        actor: UserId,
        id: Uuid,
    ) -> Result<(), FrontMatterRepositoryError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        require_admin(&mut tx, actor).await?;
        let used: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.paper_front_matter_pins WHERE front_matter_pack_id=$1) OR EXISTS(SELECT 1 FROM latex_core.programme_template_defaults WHERE front_matter_pack_id=$1) OR EXISTS(SELECT 1 FROM latex_core.institution_template_config WHERE global_fallback_front_matter_pack_id=$1)")
            .bind(id).fetch_one(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        if used {
            return Err(FrontMatterRepositoryError::InUse);
        }
        let deleted =
            sqlx::query("DELETE FROM latex_core.front_matter_pack_files WHERE pack_id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(FrontMatterRepositoryError::Database)?;
        let pack = sqlx::query("DELETE FROM latex_core.front_matter_packs WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        if pack.rows_affected() == 0 {
            return Err(FrontMatterRepositoryError::NotFound);
        }
        audit(
            &mut tx,
            actor,
            "front_matter.pack.removed",
            "front_matter_pack",
            id,
            json!({"file_count":deleted.rows_affected()}),
        )
        .await?;
        tx.commit()
            .await
            .map_err(FrontMatterRepositoryError::Database)
    }

    pub async fn team_detail(
        &self,
        actor: UserId,
        paper_id: Uuid,
    ) -> Result<Value, FrontMatterRepositoryError> {
        let row = sqlx::query(r"SELECT t.id,t.workspace_id,t.name,t.status,member.is_leader,role.role,
                    pin.front_matter_pack_id,p.name AS pack_name,p.manifest_json,pin.resolution_method,
                    pin.dominant_programme_code,pin.status AS front_matter_status,pin.missing_required_fields,pin.last_error,
                    warning.warning_code,warning.detail AS warning_detail
             FROM latex_core.paper_teams t
             LEFT JOIN latex_core.paper_team_members member ON member.paper_team_id=t.id AND member.user_id=$2
             LEFT JOIN latex_core.global_user_roles role ON role.user_id=$2
             LEFT JOIN latex_core.paper_front_matter_pins pin ON pin.paper_team_id=t.id
             LEFT JOIN latex_core.front_matter_packs p ON p.id=pin.front_matter_pack_id
             LEFT JOIN LATERAL (SELECT warning_code,detail FROM latex_core.paper_team_materialization_warnings warning WHERE warning.paper_team_id=t.id AND warning.resolved_at IS NULL ORDER BY warning.created_at DESC LIMIT 1) warning ON TRUE
             WHERE t.id=$1 AND (role.role='admin' OR member.user_id IS NOT NULL)")
            .bind(paper_id).bind(actor.as_uuid()).fetch_optional(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?.ok_or(FrontMatterRepositoryError::NotFound)?;
        let role: Option<String> = row
            .try_get("role")
            .map_err(FrontMatterRepositoryError::Database)?;
        let leader: Option<bool> = row
            .try_get("is_leader")
            .map_err(FrontMatterRepositoryError::Database)?;
        let values = sqlx::query("SELECT field_key,value_json,value_source FROM latex_core.paper_front_matter_values WHERE paper_team_id=$1 ORDER BY field_key")
            .bind(paper_id).fetch_all(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?
            .into_iter().map(|item| Ok(json!({"field_key":item.try_get::<String,_>("field_key").map_err(FrontMatterRepositoryError::Database)?,"value":item.try_get::<Value,_>("value_json").map_err(FrontMatterRepositoryError::Database)?,"value_source":item.try_get::<String,_>("value_source").map_err(FrontMatterRepositoryError::Database)?}))).collect::<Result<Vec<_>,FrontMatterRepositoryError>>()?;
        let sections = sqlx::query("SELECT section_key,enabled FROM latex_core.paper_front_matter_sections WHERE paper_team_id=$1 ORDER BY section_key")
            .bind(paper_id).fetch_all(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?
            .into_iter().map(|item| Ok(json!({"section_key":item.try_get::<String,_>("section_key").map_err(FrontMatterRepositoryError::Database)?,"enabled":item.try_get::<bool,_>("enabled").map_err(FrontMatterRepositoryError::Database)?}))).collect::<Result<Vec<_>,FrontMatterRepositoryError>>()?;
        let warning_code: Option<String> = row
            .try_get("warning_code")
            .map_err(FrontMatterRepositoryError::Database)?;
        let front_matter_status: Option<String> = row
            .try_get("front_matter_status")
            .map_err(FrontMatterRepositoryError::Database)?;
        let status = match warning_code.as_deref() {
            Some("FRONT_MATTER_TEMPLATE_INCOMPATIBLE") => "INCOMPATIBLE_TEMPLATE".into(),
            Some("FRONT_MATTER_RENDER_FAILED") => "RENDER_FAILED".into(),
            _ => front_matter_status.unwrap_or_else(|| "NONE".into()),
        };
        let pin_error: Option<String> = row
            .try_get("last_error")
            .map_err(FrontMatterRepositoryError::Database)?;
        let warning_detail: Option<String> = row
            .try_get("warning_detail")
            .map_err(FrontMatterRepositoryError::Database)?;
        Ok(json!({
            "paper_team_id":row.try_get::<Uuid,_>("id").map_err(FrontMatterRepositoryError::Database)?,
            "workspace_id":row.try_get::<Uuid,_>("workspace_id").map_err(FrontMatterRepositoryError::Database)?,
            "team_name":row.try_get::<String,_>("name").map_err(FrontMatterRepositoryError::Database)?,
            "paper_status":row.try_get::<String,_>("status").map_err(FrontMatterRepositoryError::Database)?,
            "pack_id":row.try_get::<Option<Uuid>,_>("front_matter_pack_id").map_err(FrontMatterRepositoryError::Database)?,
            "pack_name":row.try_get::<Option<String>,_>("pack_name").map_err(FrontMatterRepositoryError::Database)?,
            "manifest":row.try_get::<Option<Value>,_>("manifest_json").map_err(FrontMatterRepositoryError::Database)?,
            "resolution_method":row.try_get::<Option<String>,_>("resolution_method").map_err(FrontMatterRepositoryError::Database)?,
            "dominant_programme_code":row.try_get::<Option<String>,_>("dominant_programme_code").map_err(FrontMatterRepositoryError::Database)?,
            "status":status,
            "missing_required_fields":row.try_get::<Option<Value>,_>("missing_required_fields").map_err(FrontMatterRepositoryError::Database)?.unwrap_or_else(|| json!([])),
            "last_error":pin_error.or(warning_detail),
            "can_edit":role.as_deref()==Some("admin") || (role.as_deref()==Some("writer") && leader==Some(true)),
            "values":values,"sections":sections
        }))
    }

    pub async fn version_state(
        &self,
        paper_id: Uuid,
    ) -> Result<Option<Value>, FrontMatterRepositoryError> {
        let pin = sqlx::query("SELECT front_matter_pack_id,dominant_programme_code,resolution_method,status,missing_required_fields,last_error,pinned_at::text FROM latex_core.paper_front_matter_pins WHERE paper_team_id=$1")
            .bind(paper_id).fetch_optional(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?;
        let Some(pin) = pin else {
            return Ok(None);
        };
        let values = sqlx::query("SELECT field_key,value_json,value_source,updated_by_user_id,updated_at::text FROM latex_core.paper_front_matter_values WHERE paper_team_id=$1 ORDER BY field_key")
            .bind(paper_id).fetch_all(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?
            .into_iter().map(|row| Ok(json!({"field_key":row.try_get::<String,_>("field_key").map_err(FrontMatterRepositoryError::Database)?,"value_json":row.try_get::<Value,_>("value_json").map_err(FrontMatterRepositoryError::Database)?,"value_source":row.try_get::<String,_>("value_source").map_err(FrontMatterRepositoryError::Database)?,"updated_by_user_id":row.try_get::<Option<Uuid>,_>("updated_by_user_id").map_err(FrontMatterRepositoryError::Database)?,"updated_at":row.try_get::<String,_>("updated_at").map_err(FrontMatterRepositoryError::Database)?}))).collect::<Result<Vec<_>,FrontMatterRepositoryError>>()?;
        let sections = sqlx::query("SELECT section_key,enabled,updated_by_user_id,updated_at::text FROM latex_core.paper_front_matter_sections WHERE paper_team_id=$1 ORDER BY section_key")
            .bind(paper_id).fetch_all(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?
            .into_iter().map(|row| Ok(json!({"section_key":row.try_get::<String,_>("section_key").map_err(FrontMatterRepositoryError::Database)?,"enabled":row.try_get::<bool,_>("enabled").map_err(FrontMatterRepositoryError::Database)?,"updated_by_user_id":row.try_get::<Option<Uuid>,_>("updated_by_user_id").map_err(FrontMatterRepositoryError::Database)?,"updated_at":row.try_get::<String,_>("updated_at").map_err(FrontMatterRepositoryError::Database)?}))).collect::<Result<Vec<_>,FrontMatterRepositoryError>>()?;
        Ok(Some(json!({
            "schema_version":1,
            "pin":{"front_matter_pack_id":pin.try_get::<Uuid,_>("front_matter_pack_id").map_err(FrontMatterRepositoryError::Database)?,"dominant_programme_code":pin.try_get::<Option<String>,_>("dominant_programme_code").map_err(FrontMatterRepositoryError::Database)?,"resolution_method":pin.try_get::<String,_>("resolution_method").map_err(FrontMatterRepositoryError::Database)?,"status":pin.try_get::<String,_>("status").map_err(FrontMatterRepositoryError::Database)?,"missing_required_fields":pin.try_get::<Value,_>("missing_required_fields").map_err(FrontMatterRepositoryError::Database)?,"last_error":pin.try_get::<Option<String>,_>("last_error").map_err(FrontMatterRepositoryError::Database)?,"pinned_at":pin.try_get::<String,_>("pinned_at").map_err(FrontMatterRepositoryError::Database)?},
            "values":values,"sections":sections
        })))
    }

    pub async fn automatic_values(
        &self,
        paper_id: Uuid,
    ) -> Result<BTreeMap<String, Value>, FrontMatterRepositoryError> {
        let team = sqlx::query(r"SELECT t.name,g.academic_year,g.semester,
                    COALESCE(pin.dominant_programme_code,res.dominant_programme_code) AS dominant_programme_code
             FROM latex_core.paper_teams t
             LEFT JOIN latex_core.external_paper_team_links link ON link.paper_team_id=t.id
             LEFT JOIN vcap.paper_assignment_groups g ON g.external_team_key=link.external_team_key
             LEFT JOIN latex_core.paper_front_matter_pins pin ON pin.paper_team_id=t.id
             LEFT JOIN latex_core.paper_template_resolutions res ON res.paper_team_id=t.id WHERE t.id=$1")
            .bind(paper_id).fetch_optional(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?.ok_or(FrontMatterRepositoryError::NotFound)?;
        let people = sqlx::query(r"SELECT m.is_leader,m.writer_order,role.role,c.email,s.name AS student_name,s.reg_no,
                    f.name AS faculty_name,f.honorific,f.designation,f.faculty_id,f.dept_id,
                    (SELECT fr.school_id FROM vcap.faculty_roles fr WHERE fr.faculty_id=f.faculty_id AND fr.school_id IS NOT NULL ORDER BY fr.role_id LIMIT 1) AS school_id
             FROM latex_core.paper_team_members m
             JOIN latex_core.global_user_roles role ON role.user_id=m.user_id
             JOIN latex_core.user_credentials c ON c.user_id=m.user_id
             LEFT JOIN vcap.student_user_links sl ON sl.user_id=m.user_id AND sl.status='LINKED'
             LEFT JOIN vcap.students s ON s.reg_no=sl.reg_no
             LEFT JOIN vcap.faculty_user_links fl ON fl.user_id=m.user_id AND fl.status='LINKED'
             LEFT JOIN vcap.faculty f ON f.faculty_id=fl.faculty_id
             WHERE m.paper_team_id=$1 ORDER BY CASE WHEN role.role='writer' THEN 0 ELSE 1 END,m.writer_order NULLS LAST,m.created_at,m.user_id")
            .bind(paper_id).fetch_all(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?;
        let mut output = BTreeMap::new();
        output.insert(
            "team.name".into(),
            Value::String(
                team.try_get("name")
                    .map_err(FrontMatterRepositoryError::Database)?,
            ),
        );
        optional_string(&team, "academic_year", "team.academic_year", &mut output)?;
        optional_string(&team, "semester", "team.semester", &mut output)?;
        optional_string(
            &team,
            "dominant_programme_code",
            "team.dominant_programme_code",
            &mut output,
        )?;
        let mut writer_names = Vec::new();
        let mut registration_numbers = Vec::new();
        let mut pairs = Vec::new();
        let mut mentor_done = false;
        for row in people {
            let role: String = row
                .try_get("role")
                .map_err(FrontMatterRepositoryError::Database)?;
            if role == "writer" {
                let name = row
                    .try_get::<Option<String>, _>("student_name")
                    .map_err(FrontMatterRepositoryError::Database)?
                    .unwrap_or(
                        row.try_get::<String, _>("email")
                            .map_err(FrontMatterRepositoryError::Database)?,
                    );
                let reg = row
                    .try_get::<Option<String>, _>("reg_no")
                    .map_err(FrontMatterRepositoryError::Database)?;
                writer_names.push(Value::String(name.clone()));
                if let Some(reg) = reg.clone() {
                    registration_numbers.push(Value::String(reg.clone()));
                    pairs.push(Value::String(format!("{name} ({reg})")));
                } else {
                    pairs.push(Value::String(name.clone()));
                }
                if row
                    .try_get::<bool, _>("is_leader")
                    .map_err(FrontMatterRepositoryError::Database)?
                {
                    output.insert("leader.name".into(), Value::String(name));
                    if let Some(reg) = reg {
                        output.insert("leader.registration_number".into(), Value::String(reg));
                    }
                }
            } else if role == "mentor" && !mentor_done {
                optional_string(&row, "faculty_name", "mentor.name", &mut output)?;
                optional_string(&row, "honorific", "mentor.honorific", &mut output)?;
                optional_string(&row, "designation", "mentor.designation", &mut output)?;
                optional_string(&row, "faculty_id", "mentor.faculty_id", &mut output)?;
                let department: Option<Uuid> = row
                    .try_get("dept_id")
                    .map_err(FrontMatterRepositoryError::Database)?;
                if let Some(department) = department {
                    output.insert(
                        "department.id".into(),
                        Value::String(department.to_string()),
                    );
                }
                optional_string(&row, "school_id", "school.id", &mut output)?;
                mentor_done = true;
            }
        }
        output.insert("writers.names".into(), Value::Array(writer_names));
        output.insert(
            "writers.registration_numbers".into(),
            Value::Array(registration_numbers),
        );
        output.insert(
            "writers.names_and_registration_numbers".into(),
            Value::Array(pairs),
        );
        Ok(output)
    }

    pub async fn team_build_identity(
        &self,
        paper_id: Uuid,
    ) -> Result<(UserId, TenantId), FrontMatterRepositoryError> {
        let row = sqlx::query(
            "SELECT member.user_id,user_record.tenant_id \
             FROM latex_core.paper_team_members member \
             JOIN latex_core.global_user_roles role ON role.user_id=member.user_id AND role.role='writer' \
             JOIN latex_core.users user_record ON user_record.id=member.user_id \
             WHERE member.paper_team_id=$1 AND member.is_leader=TRUE LIMIT 1",
        )
        .bind(paper_id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(FrontMatterRepositoryError::Database)?
        .ok_or(FrontMatterRepositoryError::NotFound)?;
        Ok((
            UserId::from_uuid(
                row.try_get("user_id")
                    .map_err(FrontMatterRepositoryError::Database)?,
            ),
            TenantId::from_uuid(
                row.try_get("tenant_id")
                    .map_err(FrontMatterRepositoryError::Database)?,
            ),
        ))
    }

    pub async fn apply_render(
        &self,
        actor: UserId,
        request: &ApplyFrontMatterRequest,
    ) -> Result<u64, FrontMatterRepositoryError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        require_front_matter_editor(&mut tx, actor, request.paper_team_id).await?;
        let row = sqlx::query("SELECT t.workspace_id,template.front_matter_compatible FROM latex_core.paper_teams t JOIN latex_core.paper_template_pins pin ON pin.paper_id=t.id JOIN latex_core.templates template ON template.id=pin.template_id WHERE t.id=$1 FOR UPDATE OF t")
            .bind(request.paper_team_id).fetch_optional(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?.ok_or(FrontMatterRepositoryError::NotFound)?;
        let workspace: Uuid = row
            .try_get("workspace_id")
            .map_err(FrontMatterRepositoryError::Database)?;
        if workspace != *request.workspace_id.as_uuid() {
            return Err(FrontMatterRepositoryError::Integrity(
                "paper workspace mismatch".into(),
            ));
        }
        if !row
            .try_get::<bool, _>("front_matter_compatible")
            .map_err(FrontMatterRepositoryError::Database)?
        {
            return Err(FrontMatterRepositoryError::IncompatibleTemplate);
        }
        let head: i64 = sqlx::query_scalar("SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1 FOR UPDATE")
            .bind(workspace).fetch_one(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        if u64::try_from(head).ok() != Some(request.expected_workspace_version) {
            return Err(FrontMatterRepositoryError::VersionConflict);
        }
        if let Some(safety) = &request.safety {
            let number: i64 = sqlx::query_scalar("SELECT COALESCE(max(version_number),0)+1 FROM latex_core.paper_versions WHERE workspace_id=$1").bind(workspace).fetch_one(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
            sqlx::query("INSERT INTO latex_core.paper_versions (id,paper_id,workspace_id,document_epoch,version_number,version_type,name,created_by_user_id,workspace_version,snapshot_id,manifest,state_hash) VALUES ($1,$2,$3,$4,$5,'front_matter_update','PRE_FRONT_MATTER_UPDATE',$6,$7,$8,$9,$10)")
                .bind(Uuid::new_v4()).bind(request.paper_team_id).bind(workspace).bind(to_i64(safety.document_epoch)?).bind(number).bind(actor.as_uuid()).bind(to_i64(safety.workspace_version)?).bind(&safety.snapshot_id).bind(&safety.manifest).bind(&safety.state_hash)
                .execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        }
        let existing = sqlx::query("SELECT f.file_id,f.path,COALESCE(p.policy,'EDITABLE') AS policy FROM latex_core.paper_files f LEFT JOIN latex_core.paper_file_policies p ON p.file_id=f.file_id WHERE f.workspace_id=$1 AND NOT f.tombstoned AND f.path LIKE '.latex-core/frontmatter/%' FOR UPDATE OF f")
            .bind(workspace).fetch_all(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        if existing
            .iter()
            .any(|row| row.try_get::<String, _>("policy").ok().as_deref() != Some("HIDDEN_SYSTEM"))
        {
            return Err(FrontMatterRepositoryError::Integrity(
                "managed Front Matter subtree contains a non-system file".into(),
            ));
        }
        let mut by_path = existing
            .into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("path")
                        .map_err(FrontMatterRepositoryError::Database)?,
                    row.try_get::<Uuid, _>("file_id")
                        .map_err(FrontMatterRepositoryError::Database)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, FrontMatterRepositoryError>>()?;
        let requested = request
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let mut operations = Vec::new();
        for (path, file_id) in &by_path {
            if !requested.contains(path.as_str()) {
                sqlx::query("UPDATE latex_core.paper_files SET tombstoned=TRUE,tombstoned_at=statement_timestamp(),revision=revision+1,updated_at=statement_timestamp() WHERE file_id=$1").bind(file_id).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
                operations.push(json!({"op":"delete_file","path":path}));
            }
        }
        for file in &request.files {
            if !file.path.starts_with(MANAGED_PREFIX) {
                return Err(FrontMatterRepositoryError::Integrity(
                    "render escaped managed subtree".into(),
                ));
            }
            let size = to_i64(file.size_bytes)?;
            let file_id = if let Some(file_id) = by_path.remove(&file.path) {
                sqlx::query("UPDATE latex_core.paper_files SET revision=revision+1,updated_at=statement_timestamp() WHERE file_id=$1").bind(file_id).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
                file_id
            } else {
                let file_id = Uuid::new_v4();
                sqlx::query("INSERT INTO latex_core.paper_files (file_id,workspace_id,path) VALUES ($1,$2,$3)").bind(file_id).bind(workspace).bind(&file.path).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
                file_id
            };
            sqlx::query("INSERT INTO latex_core.paper_file_policies (file_id,workspace_id,policy,updated_by_admin_user_id) VALUES ($1,$2,'HIDDEN_SYSTEM',$3) ON CONFLICT(file_id) DO UPDATE SET policy='HIDDEN_SYSTEM',updated_by_admin_user_id=EXCLUDED.updated_by_admin_user_id,updated_at=statement_timestamp()")
                .bind(file_id).bind(workspace).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
            operations.push(json!({"op":"put_file","path":file.path,"blob_hash":file.blob_hash,"size_bytes":size}));
        }
        let next = head.checked_add(1).ok_or_else(|| {
            FrontMatterRepositoryError::Integrity("workspace version overflow".into())
        })?;
        sqlx::query("INSERT INTO latex_core.workspace_events (workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) VALUES ($1,$2,$3,$4,'workspace.mutation',1,$5,$6)")
            .bind(workspace).bind(next).bind(Uuid::new_v4()).bind(head).bind(json!({"schema_version":1,"operations":operations})).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        sqlx::query("UPDATE latex_core.workspace_heads SET durable_version=$2,updated_at=statement_timestamp() WHERE workspace_id=$1").bind(workspace).bind(next).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        sqlx::query("INSERT INTO latex_core.paper_front_matter_pins (paper_team_id,front_matter_pack_id,dominant_programme_code,resolution_method,assigned_by_user_id,status,missing_required_fields,last_error) VALUES ($1,$2,$3,$4,$5,'READY','[]',NULL) ON CONFLICT(paper_team_id) DO UPDATE SET front_matter_pack_id=EXCLUDED.front_matter_pack_id,dominant_programme_code=EXCLUDED.dominant_programme_code,resolution_method=EXCLUDED.resolution_method,assigned_by_user_id=EXCLUDED.assigned_by_user_id,status='READY',missing_required_fields='[]',last_error=NULL,pinned_at=statement_timestamp()")
            .bind(request.paper_team_id).bind(request.pack_id).bind(&request.dominant_programme_code).bind(&request.resolution_method).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        sqlx::query("DELETE FROM latex_core.paper_front_matter_values WHERE paper_team_id=$1")
            .bind(request.paper_team_id)
            .execute(&mut *tx)
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        for value in &request.values {
            sqlx::query("INSERT INTO latex_core.paper_front_matter_values (paper_team_id,field_key,value_json,value_source,updated_by_user_id) VALUES ($1,$2,$3,$4,$5)").bind(request.paper_team_id).bind(&value.field_key).bind(&value.value_json).bind(&value.value_source).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        }
        sqlx::query("DELETE FROM latex_core.paper_front_matter_sections WHERE paper_team_id=$1")
            .bind(request.paper_team_id)
            .execute(&mut *tx)
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        for (key, enabled) in &request.sections {
            sqlx::query("INSERT INTO latex_core.paper_front_matter_sections (paper_team_id,section_key,enabled,updated_by_user_id) VALUES ($1,$2,$3,$4)").bind(request.paper_team_id).bind(key).bind(enabled).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        }
        sqlx::query("UPDATE latex_core.paper_team_materialization_warnings SET resolved_at=statement_timestamp() WHERE paper_team_id=$1 AND resolved_at IS NULL AND warning_code IN ('FRONT_MATTER_TEMPLATE_INCOMPATIBLE','FRONT_MATTER_RENDER_FAILED')")
            .bind(request.paper_team_id).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        audit(&mut tx, actor, "front_matter.rendered", "paper_team", request.paper_team_id, json!({"pack_id":request.pack_id,"workspace_version":next,"file_count":request.files.len(),"resolution_method":request.resolution_method})).await?;
        tx.commit()
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        tracing::info!(paper_id=%request.paper_team_id, workspace_id=%request.workspace_id, pack_id=%request.pack_id, workspace_version=next, "Front Matter render committed");
        u64::try_from(next)
            .map_err(|_| FrontMatterRepositoryError::Integrity("negative workspace version".into()))
    }

    pub async fn remove_render(
        &self,
        actor: UserId,
        paper_id: Uuid,
        workspace_id: WorkspaceId,
        expected: u64,
        empty_blob: BlobHash,
        safety: Option<ExactStateRecord>,
    ) -> Result<u64, FrontMatterRepositoryError> {
        let request = ApplyFrontMatterRequest {
            paper_team_id: paper_id,
            workspace_id,
            expected_workspace_version: expected,
            pack_id: Uuid::nil(),
            dominant_programme_code: None,
            resolution_method: "MANUAL_OVERRIDE".into(),
            files: vec![ManagedFrontMatterFile {
                path: format!("{MANAGED_PREFIX}frontmatter.tex"),
                blob_hash: empty_blob,
                size_bytes: 0,
            }],
            values: Vec::new(),
            sections: BTreeMap::new(),
            safety,
        };
        // A temporary sentinel satisfies the shared mutation path, then the pin is
        // removed in a second short transaction. The managed empty file remains.
        let version = self.apply_remove_render(actor, &request).await?;
        Ok(version)
    }

    async fn apply_remove_render(
        &self,
        actor: UserId,
        request: &ApplyFrontMatterRequest,
    ) -> Result<u64, FrontMatterRepositoryError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        require_admin(&mut tx, actor).await?;
        let workspace: Uuid = sqlx::query_scalar(
            "SELECT workspace_id FROM latex_core.paper_teams WHERE id=$1 FOR UPDATE",
        )
        .bind(request.paper_team_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(FrontMatterRepositoryError::Database)?
        .ok_or(FrontMatterRepositoryError::NotFound)?;
        if workspace != *request.workspace_id.as_uuid() {
            return Err(FrontMatterRepositoryError::Integrity(
                "paper workspace mismatch".into(),
            ));
        }
        let head: i64 = sqlx::query_scalar("SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1 FOR UPDATE").bind(workspace).fetch_one(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        if u64::try_from(head).ok() != Some(request.expected_workspace_version) {
            return Err(FrontMatterRepositoryError::VersionConflict);
        }
        if let Some(safety) = &request.safety {
            let number:i64=sqlx::query_scalar("SELECT COALESCE(max(version_number),0)+1 FROM latex_core.paper_versions WHERE workspace_id=$1").bind(workspace).fetch_one(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
            sqlx::query("INSERT INTO latex_core.paper_versions (id,paper_id,workspace_id,document_epoch,version_number,version_type,name,created_by_user_id,workspace_version,snapshot_id,manifest,state_hash) VALUES ($1,$2,$3,$4,$5,'front_matter_update','PRE_FRONT_MATTER_REMOVAL',$6,$7,$8,$9,$10)").bind(Uuid::new_v4()).bind(request.paper_team_id).bind(workspace).bind(to_i64(safety.document_epoch)?).bind(number).bind(actor.as_uuid()).bind(to_i64(safety.workspace_version)?).bind(&safety.snapshot_id).bind(&safety.manifest).bind(&safety.state_hash).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        }
        let rows=sqlx::query("SELECT file_id,path,COALESCE(p.policy,'EDITABLE') AS policy FROM latex_core.paper_files f LEFT JOIN latex_core.paper_file_policies p ON p.file_id=f.file_id WHERE f.workspace_id=$1 AND NOT f.tombstoned AND f.path LIKE '.latex-core/frontmatter/%' FOR UPDATE OF f").bind(workspace).fetch_all(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        if rows
            .iter()
            .any(|row| row.try_get::<String, _>("policy").ok().as_deref() != Some("HIDDEN_SYSTEM"))
        {
            return Err(FrontMatterRepositoryError::Integrity(
                "managed Front Matter subtree contains a non-system file".into(),
            ));
        }
        let mut operations = Vec::new();
        let mut entry_id = None;
        for row in rows {
            let id: Uuid = row
                .try_get("file_id")
                .map_err(FrontMatterRepositoryError::Database)?;
            let path: String = row
                .try_get("path")
                .map_err(FrontMatterRepositoryError::Database)?;
            if path.ends_with("/frontmatter.tex") {
                entry_id = Some(id);
            } else {
                sqlx::query("UPDATE latex_core.paper_files SET tombstoned=TRUE,tombstoned_at=statement_timestamp(),revision=revision+1,updated_at=statement_timestamp() WHERE file_id=$1").bind(id).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
                operations.push(json!({"op":"delete_file","path":path}));
            }
        }
        let entry_path = format!("{MANAGED_PREFIX}frontmatter.tex");
        let id = entry_id.unwrap_or_else(Uuid::new_v4);
        if entry_id.is_some() {
            sqlx::query("UPDATE latex_core.paper_files SET revision=revision+1,updated_at=statement_timestamp() WHERE file_id=$1").bind(id).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        } else {
            sqlx::query(
                "INSERT INTO latex_core.paper_files(file_id,workspace_id,path) VALUES($1,$2,$3)",
            )
            .bind(id)
            .bind(workspace)
            .bind(&entry_path)
            .execute(&mut *tx)
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        }
        sqlx::query("INSERT INTO latex_core.paper_file_policies(file_id,workspace_id,policy,updated_by_admin_user_id) VALUES($1,$2,'HIDDEN_SYSTEM',$3) ON CONFLICT(file_id) DO UPDATE SET policy='HIDDEN_SYSTEM',updated_by_admin_user_id=EXCLUDED.updated_by_admin_user_id,updated_at=statement_timestamp()").bind(id).bind(workspace).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        operations.push(json!({"op":"put_file","path":entry_path,"blob_hash":request.files[0].blob_hash,"size_bytes":0}));
        let next = head + 1;
        sqlx::query("INSERT INTO latex_core.workspace_events(workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) VALUES($1,$2,$3,$4,'workspace.mutation',1,$5,$6)").bind(workspace).bind(next).bind(Uuid::new_v4()).bind(head).bind(json!({"schema_version":1,"operations":operations})).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        sqlx::query("UPDATE latex_core.workspace_heads SET durable_version=$2,updated_at=statement_timestamp() WHERE workspace_id=$1").bind(workspace).bind(next).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        sqlx::query("DELETE FROM latex_core.paper_front_matter_sections WHERE paper_team_id=$1")
            .bind(request.paper_team_id)
            .execute(&mut *tx)
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        sqlx::query("DELETE FROM latex_core.paper_front_matter_values WHERE paper_team_id=$1")
            .bind(request.paper_team_id)
            .execute(&mut *tx)
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        sqlx::query("DELETE FROM latex_core.paper_front_matter_pins WHERE paper_team_id=$1")
            .bind(request.paper_team_id)
            .execute(&mut *tx)
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        sqlx::query("UPDATE latex_core.paper_team_materialization_warnings SET resolved_at=statement_timestamp() WHERE paper_team_id=$1 AND resolved_at IS NULL AND warning_code IN ('FRONT_MATTER_TEMPLATE_INCOMPATIBLE','FRONT_MATTER_RENDER_FAILED')")
            .bind(request.paper_team_id).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
        audit(
            &mut tx,
            actor,
            "front_matter.removed",
            "paper_team",
            request.paper_team_id,
            json!({"workspace_version":next}),
        )
        .await?;
        tx.commit()
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        u64::try_from(next)
            .map_err(|_| FrontMatterRepositoryError::Integrity("negative workspace version".into()))
    }

    pub async fn resolve_default(
        &self,
        dominant_programme: Option<&str>,
    ) -> Result<Option<(Uuid, String)>, FrontMatterRepositoryError> {
        if let Some(programme) = dominant_programme {
            let mapped:Option<Uuid>=sqlx::query_scalar("SELECT front_matter_pack_id FROM latex_core.programme_template_defaults WHERE programme_code=$1").bind(programme).fetch_optional(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?.flatten();
            if let Some(pack) = mapped {
                return Ok(Some((pack, "PROGRAMME_DEFAULT".into())));
            }
        }
        let fallback:Option<Uuid>=sqlx::query_scalar("SELECT global_fallback_front_matter_pack_id FROM latex_core.institution_template_config WHERE singleton").fetch_one(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?;
        Ok(fallback.map(|pack| (pack, "GLOBAL_FALLBACK".into())))
    }

    pub async fn enqueue_affected_for_import(
        &self,
        job_id: Uuid,
    ) -> Result<u64, FrontMatterRepositoryError> {
        let result=sqlx::query(r"INSERT INTO latex_core.front_matter_rerender_queue(paper_team_id,reason)
            SELECT DISTINCT pin.paper_team_id,'INSTITUTION_PERSON_UPDATED'
            FROM latex_core.paper_front_matter_pins pin
            JOIN latex_core.paper_team_members member ON member.paper_team_id=pin.paper_team_id
            LEFT JOIN vcap.student_user_links student ON student.user_id=member.user_id AND student.status='LINKED'
            LEFT JOIN vcap.faculty_user_links faculty ON faculty.user_id=member.user_id AND faculty.status='LINKED'
            WHERE EXISTS (
                SELECT 1 FROM latex_core.institution_import_rows row
                JOIN latex_core.institution_import_jobs source_job ON source_job.id=row.job_id
                WHERE (row.job_id=$1 OR source_job.batch_id=(SELECT batch_id FROM latex_core.institution_import_jobs WHERE id=$1))
                  AND row.status='APPLIED' AND row.action='UPDATE'
                  AND ((row.source_table_or_sheet='students' AND row.payload->>'reg_no'=student.reg_no)
                    OR (row.source_table_or_sheet='faculty' AND row.payload->>'faculty_id'=faculty.faculty_id))
            )
            ON CONFLICT(paper_team_id) DO UPDATE SET reason=EXCLUDED.reason,state='QUEUED',available_at=statement_timestamp(),claimed_at=NULL,last_error=NULL,updated_at=statement_timestamp()")
            .bind(job_id).execute(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?;
        Ok(result.rows_affected())
    }

    pub async fn claim_rerender(&self) -> Result<Option<Uuid>, FrontMatterRepositoryError> {
        sqlx::query_scalar(r"WITH selected AS (
                SELECT paper_team_id FROM latex_core.front_matter_rerender_queue
                WHERE (state='QUEUED' AND available_at<=statement_timestamp())
                   OR (state='CLAIMED' AND claimed_at<statement_timestamp()-interval '10 minutes')
                ORDER BY available_at,updated_at,paper_team_id FOR UPDATE SKIP LOCKED LIMIT 1
            ) UPDATE latex_core.front_matter_rerender_queue queue
              SET state='CLAIMED',attempts=attempts+1,claimed_at=statement_timestamp(),updated_at=statement_timestamp()
              FROM selected WHERE queue.paper_team_id=selected.paper_team_id
              RETURNING queue.paper_team_id")
            .fetch_optional(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)
    }

    pub async fn finish_rerender(
        &self,
        paper_id: Uuid,
        succeeded: bool,
    ) -> Result<(), FrontMatterRepositoryError> {
        if succeeded {
            sqlx::query(
                "DELETE FROM latex_core.front_matter_rerender_queue WHERE paper_team_id=$1",
            )
            .bind(paper_id)
            .execute(self.database.pool())
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
        } else {
            let mut tx = self
                .database
                .pool()
                .begin()
                .await
                .map_err(FrontMatterRepositoryError::Database)?;
            sqlx::query("UPDATE latex_core.front_matter_rerender_queue SET state=CASE WHEN attempts>=5 THEN 'FAILED' ELSE 'QUEUED' END,available_at=statement_timestamp()+interval '5 minutes',claimed_at=NULL,last_error='Front Matter rerender failed; existing files were preserved',updated_at=statement_timestamp() WHERE paper_team_id=$1")
                .bind(paper_id).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
            sqlx::query("UPDATE latex_core.paper_front_matter_pins SET status='RENDER_FAILED',last_error='Automatic Front Matter refresh failed; existing rendered files were preserved' WHERE paper_team_id=$1")
                .bind(paper_id).execute(&mut *tx).await.map_err(FrontMatterRepositoryError::Database)?;
            tx.commit()
                .await
                .map_err(FrontMatterRepositoryError::Database)?;
        }
        Ok(())
    }

    pub async fn record_materialization_warning(
        &self,
        paper_id: Uuid,
        code: &str,
        detail: &str,
    ) -> Result<(), FrontMatterRepositoryError> {
        sqlx::query("INSERT INTO latex_core.paper_team_materialization_warnings (id,paper_team_id,warning_code,detail) VALUES ($1,$2,$3,$4)")
            .bind(Uuid::new_v4()).bind(paper_id).bind(code).bind(detail).execute(self.database.pool()).await.map_err(FrontMatterRepositoryError::Database)?;
        Ok(())
    }
}

fn decode_pack(
    row: sqlx::postgres::PgRow,
) -> Result<FrontMatterPackRecord, FrontMatterRepositoryError> {
    Ok(FrontMatterPackRecord {
        id: row
            .try_get("id")
            .map_err(FrontMatterRepositoryError::Database)?,
        name: row
            .try_get("name")
            .map_err(FrontMatterRepositoryError::Database)?,
        description: row
            .try_get("description")
            .map_err(FrontMatterRepositoryError::Database)?,
        manifest_json: row
            .try_get("manifest_json")
            .map_err(FrontMatterRepositoryError::Database)?,
        content_hash: row
            .try_get("content_hash")
            .map_err(FrontMatterRepositoryError::Database)?,
        created_by_user_id: UserId::from_uuid(
            row.try_get("created_by_user_id")
                .map_err(FrontMatterRepositoryError::Database)?,
        ),
        created_at: row
            .try_get("created_at")
            .map_err(FrontMatterRepositoryError::Database)?,
        archived_at: row
            .try_get("archived_at")
            .map_err(FrontMatterRepositoryError::Database)?,
        usage_count: row
            .try_get("usage_count")
            .map_err(FrontMatterRepositoryError::Database)?,
    })
}

async fn require_admin(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: UserId,
) -> Result<(), FrontMatterRepositoryError> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM latex_core.global_user_roles WHERE user_id=$1")
            .bind(actor.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(FrontMatterRepositoryError::Database)?;
    if role.as_deref() == Some("admin") {
        Ok(())
    } else {
        Err(FrontMatterRepositoryError::Forbidden)
    }
}
async fn require_front_matter_editor(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: UserId,
    paper_id: Uuid,
) -> Result<(), FrontMatterRepositoryError> {
    let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.global_user_roles WHERE user_id=$1 AND role='admin') OR EXISTS(SELECT 1 FROM latex_core.paper_team_members m JOIN latex_core.global_user_roles r ON r.user_id=m.user_id WHERE m.paper_team_id=$2 AND m.user_id=$1 AND m.is_leader AND r.role='writer')").bind(actor.as_uuid()).bind(paper_id).fetch_one(&mut **tx).await.map_err(FrontMatterRepositoryError::Database)?;
    if allowed {
        Ok(())
    } else {
        Err(FrontMatterRepositoryError::Forbidden)
    }
}
async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: UserId,
    event: &str,
    resource: &str,
    id: Uuid,
    metadata: Value,
) -> Result<(), FrontMatterRepositoryError> {
    sqlx::query("INSERT INTO latex_core.audit_events(id,actor_user_id,event_type,resource_type,resource_id,metadata) VALUES($1,$2,$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(actor.as_uuid()).bind(event).bind(resource).bind(id).bind(metadata).execute(&mut **tx).await.map_err(FrontMatterRepositoryError::Database)?;
    Ok(())
}
fn map_conflict(error: sqlx::Error) -> FrontMatterRepositoryError {
    if matches!(error.as_database_error().and_then(sqlx::error::DatabaseError::code),Some(code) if code=="23505")
    {
        FrontMatterRepositoryError::InUse
    } else {
        FrontMatterRepositoryError::Database(error)
    }
}
fn to_i64(value: u64) -> Result<i64, FrontMatterRepositoryError> {
    i64::try_from(value).map_err(|_| {
        FrontMatterRepositoryError::Integrity("value exceeds PostgreSQL BIGINT".into())
    })
}
fn optional_string(
    row: &sqlx::postgres::PgRow,
    column: &str,
    key: &str,
    output: &mut BTreeMap<String, Value>,
) -> Result<(), FrontMatterRepositoryError> {
    let value: Option<String> = row
        .try_get(column)
        .map_err(FrontMatterRepositoryError::Database)?;
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        output.insert(key.into(), Value::String(value));
    }
    Ok(())
}
