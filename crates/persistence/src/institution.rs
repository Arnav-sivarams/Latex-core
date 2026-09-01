//! V2.2 institutional source-of-record, bounded file parsing, and import application.

use crate::Database;
use calamine::{Data, Reader, Xlsx};
use core_types::UserId;
use csv::StringRecord;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, QueryBuilder, Row, postgres::PgRow};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::Cursor,
    str::FromStr,
};
use thiserror::Error;
use uuid::Uuid;

pub const INSTITUTION_TABLES: [&str; 13] = [
    "departments",
    "admins",
    "faculty",
    "programmes",
    "schools",
    "students",
    "student_course_registrations",
    "faculty_guide_capacity",
    "department_roles",
    "faculty_roles",
    "paper_teams",
    "paper_team_writers",
    "paper_team_mentors",
];

const APPLY_ORDER: [&str; 13] = [
    "departments",
    "schools",
    "admins",
    "faculty",
    "programmes",
    "students",
    "student_course_registrations",
    "faculty_guide_capacity",
    "department_roles",
    "faculty_roles",
    "paper_teams",
    "paper_team_writers",
    "paper_team_mentors",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ImportMode {
    ValidateOnly,
    Merge,
    AddOnly,
}

impl ImportMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ValidateOnly => "VALIDATE_ONLY",
            Self::Merge => "MERGE",
            Self::AddOnly => "ADD_ONLY",
        }
    }
}

impl FromStr for ImportMode {
    type Err = InstitutionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_uppercase().as_str() {
            "VALIDATE_ONLY" => Ok(Self::ValidateOnly),
            "MERGE" => Ok(Self::Merge),
            "ADD_ONLY" => Ok(Self::AddOnly),
            _ => Err(InstitutionError::InvalidInput("unknown import mode".into())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportFileType {
    Csv,
    Xlsx,
}

impl ImportFileType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::Xlsx => "XLSX",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ImportLimits {
    pub max_upload_bytes: usize,
    pub max_worksheets: usize,
    pub max_rows_per_sheet: usize,
    pub max_columns: usize,
    pub max_cell_characters: usize,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            max_upload_bytes: 32 * 1024 * 1024,
            max_worksheets: 20,
            max_rows_per_sheet: 100_000,
            max_columns: 128,
            max_cell_characters: 16_384,
        }
    }
}

#[derive(Debug, Error)]
pub enum InstitutionError {
    #[error("invalid institution import: {0}")]
    InvalidInput(String),
    #[error("institution import not found")]
    NotFound,
    #[error("institution import state conflict: {0}")]
    Conflict(String),
    #[error("institution import database operation failed")]
    Database(#[source] sqlx::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstitutionImportJob {
    pub id: Uuid,
    pub import_kind: String,
    pub mode: String,
    pub original_filename: String,
    pub content_sha256: String,
    pub file_type: String,
    pub submitted_by_user_id: Uuid,
    pub status: String,
    pub total_rows: i64,
    pub inserted_rows: i64,
    pub updated_rows: i64,
    pub skipped_rows: i64,
    pub error_rows: i64,
    pub created_at: String,
    pub validated_at: Option<String>,
    pub applied_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstitutionImportRow {
    pub job_id: Uuid,
    pub source_table_or_sheet: String,
    pub row_number: i64,
    pub natural_key: Value,
    pub payload: Value,
    pub action: String,
    pub status: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemplateResolution {
    pub selected_template_id: Uuid,
    pub dominant_programme_code: Option<String>,
    pub resolution_method: String,
    pub counts: BTreeMap<String, usize>,
    pub tie_break: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProgrammeTemplateDefault {
    pub programme_code: String,
    pub template_id: Option<Uuid>,
    pub template_name: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportedTeamPlan {
    pub external_team_key: String,
    pub team_name: String,
    pub existing_paper_team_id: Option<Uuid>,
    pub writer_user_ids: Vec<Uuid>,
    pub leader_user_id: Option<Uuid>,
    pub mentor_user_ids: Vec<Uuid>,
    pub unresolved: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PaperTeamPageFilter {
    pub limit: i64,
    pub page: i64,
    pub search: Option<String>,
    pub status: Option<String>,
    pub programme_code: Option<String>,
    pub mentor_user_id: Option<Uuid>,
    pub leader_user_id: Option<Uuid>,
    pub template_id: Option<Uuid>,
    pub review_state: Option<String>,
    pub source: Option<String>,
    pub unresolved: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperTeamPage {
    pub total: i64,
    pub has_more: bool,
    pub page: i64,
    pub limit: i64,
    pub items: Vec<Value>,
}

#[derive(Clone, Debug)]
struct SourceRow {
    table: String,
    row_number: i64,
    natural_key: Value,
    payload: Value,
    error: Option<(&'static str, String)>,
}

#[derive(Clone, Debug)]
pub struct InstitutionRepository {
    database: Database,
}

impl InstitutionRepository {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn validate_upload(
        &self,
        submitted_by: UserId,
        filename: &str,
        target_table: Option<&str>,
        mode: ImportMode,
        bytes: &[u8],
        limits: ImportLimits,
    ) -> Result<InstitutionImportJob, InstitutionError> {
        if filename.trim().is_empty() || filename.chars().count() > 255 {
            return Err(InstitutionError::InvalidInput("invalid filename".into()));
        }
        if bytes.is_empty() || bytes.len() > limits.max_upload_bytes {
            return Err(InstitutionError::InvalidInput(
                "upload is empty or exceeds the configured byte limit".into(),
            ));
        }
        let file_type = file_type(filename)?;
        let mut rows = match file_type {
            ImportFileType::Csv => parse_csv(filename, target_table, bytes, limits)?,
            ImportFileType::Xlsx => {
                if target_table.is_some() {
                    return Err(InstitutionError::InvalidInput(
                        "XLSX target table comes from worksheet names".into(),
                    ));
                }
                parse_xlsx(bytes, limits)?
            }
        };
        validate_cross_rows(&mut rows);
        self.validate_database_references(&mut rows).await?;
        self.assign_actions(&mut rows, mode).await?;

        let id = Uuid::new_v4();
        let digest = hex::encode(Sha256::digest(bytes));
        let import_kind = if file_type == ImportFileType::Csv {
            rows.first()
                .map_or_else(|| "empty".to_owned(), |row| row.table.clone())
        } else {
            "workbook".to_owned()
        };
        let total_rows = i64::try_from(rows.len())
            .map_err(|_| InstitutionError::InvalidInput("too many rows".into()))?;
        let error_rows = i64::try_from(rows.iter().filter(|row| row.error.is_some()).count())
            .map_err(|_| InstitutionError::InvalidInput("too many errors".into()))?;
        let status = if error_rows == 0 {
            "VALIDATED"
        } else {
            "FAILED"
        };
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        sqlx::query(
            "INSERT INTO latex_core.institution_import_jobs \
             (id,import_kind,mode,original_filename,content_sha256,file_type,submitted_by_user_id,status,total_rows,error_rows,validated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,statement_timestamp())",
        )
        .bind(id)
        .bind(&import_kind)
        .bind(mode.as_str())
        .bind(filename)
        .bind(&digest)
        .bind(file_type.as_str())
        .bind(submitted_by.as_uuid())
        .bind(status)
        .bind(total_rows)
        .bind(error_rows)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;

        for chunk in rows.chunks(500) {
            let mut builder: QueryBuilder<'_, Postgres> = QueryBuilder::new(
                "INSERT INTO latex_core.institution_import_rows \
                 (job_id,source_table_or_sheet,row_number,natural_key,payload,action,status,error_code,error_message) ",
            );
            builder.push_values(chunk, |mut separated, row| {
                let (code, message) = row
                    .error
                    .as_ref()
                    .map_or((None, None), |(code, message)| (Some(*code), Some(message)));
                separated
                    .push_bind(id)
                    .push_bind(&row.table)
                    .push_bind(row.row_number)
                    .push_bind(&row.natural_key)
                    .push_bind(&row.payload)
                    .push_bind(if row.error.is_some() {
                        "INVALID"
                    } else {
                        action_for(row)
                    })
                    .push_bind(if row.error.is_some() {
                        "ERROR"
                    } else {
                        "VALID"
                    })
                    .push_bind(code)
                    .push_bind(message);
            });
            builder
                .build()
                .execute(&mut *tx)
                .await
                .map_err(InstitutionError::Database)?;
        }
        audit_tx(
            &mut tx,
            submitted_by,
            "institution.import.uploaded",
            "institution_import_job",
            id,
            json!({"filename": filename, "sha256": digest, "file_type": file_type.as_str()}),
        )
        .await?;
        audit_tx(
            &mut tx,
            submitted_by,
            "institution.import.validated",
            "institution_import_job",
            id,
            json!({"total_rows": total_rows, "error_rows": error_rows, "status": status}),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)?;
        self.job(id).await
    }

    pub async fn apply_import(
        &self,
        job_id: Uuid,
        actor: UserId,
    ) -> Result<InstitutionImportJob, InstitutionError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        let job = sqlx::query(
            "SELECT mode,status,error_rows FROM latex_core.institution_import_jobs WHERE id=$1 FOR UPDATE",
        )
        .bind(job_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?
        .ok_or(InstitutionError::NotFound)?;
        let mode: String = job.try_get("mode").map_err(InstitutionError::Database)?;
        let status: String = job.try_get("status").map_err(InstitutionError::Database)?;
        let error_rows: i64 = job
            .try_get("error_rows")
            .map_err(InstitutionError::Database)?;
        if mode == "VALIDATE_ONLY" {
            return Err(InstitutionError::Conflict(
                "VALIDATE_ONLY jobs cannot be applied".into(),
            ));
        }
        if status == "APPLIED" || status == "PARTIAL" {
            tx.rollback().await.map_err(InstitutionError::Database)?;
            return self.job(job_id).await;
        }
        if status != "VALIDATED" || error_rows != 0 {
            return Err(InstitutionError::Conflict(
                "only error-free VALIDATED jobs can be applied".into(),
            ));
        }
        sqlx::query("UPDATE latex_core.institution_import_jobs SET status='APPLYING' WHERE id=$1")
            .bind(job_id)
            .execute(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?;

        let merge = mode == "MERGE";
        for table in APPLY_ORDER {
            let payloads: Vec<Value> = sqlx::query_scalar(
                "SELECT payload FROM latex_core.institution_import_rows \
                 WHERE job_id=$1 AND source_table_or_sheet=$2 AND status='VALID' ORDER BY row_number",
            )
            .bind(job_id)
            .bind(table)
            .fetch_all(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?;
            if !payloads.is_empty() {
                apply_table(&mut tx, table, merge, Value::Array(payloads)).await?;
            }
        }
        reconcile_identity_links_tx(&mut tx, actor).await?;
        let inserted: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.institution_import_rows WHERE job_id=$1 AND action='INSERT'",
        )
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        let updated: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.institution_import_rows WHERE job_id=$1 AND action='UPDATE'",
        )
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        let skipped: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.institution_import_rows WHERE job_id=$1 AND action='SKIP'",
        )
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        sqlx::query(
            "UPDATE latex_core.institution_import_rows SET status=CASE WHEN action='SKIP' THEN 'SKIPPED' ELSE 'APPLIED' END WHERE job_id=$1 AND status='VALID'",
        )
        .bind(job_id)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        sqlx::query(
            "UPDATE latex_core.institution_import_jobs SET status='APPLIED',inserted_rows=$2,updated_rows=$3,skipped_rows=$4,applied_at=statement_timestamp() WHERE id=$1",
        )
        .bind(job_id)
        .bind(inserted)
        .bind(updated)
        .bind(skipped)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        audit_tx(
            &mut tx,
            actor,
            "institution.import.applied",
            "institution_import_job",
            job_id,
            json!({"inserted_rows": inserted, "updated_rows": updated, "skipped_rows": skipped}),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)?;
        self.job(job_id).await
    }

    pub async fn list_jobs(
        &self,
        limit: i64,
        before: Option<Uuid>,
    ) -> Result<Vec<InstitutionImportJob>, InstitutionError> {
        let limit = limit.clamp(1, 200);
        let rows = sqlx::query(
            "SELECT j.id,j.import_kind,j.mode,j.original_filename,j.content_sha256,j.file_type,j.submitted_by_user_id,j.status,j.total_rows,j.inserted_rows,j.updated_rows,j.skipped_rows,j.error_rows,j.created_at::text,j.validated_at::text,j.applied_at::text \
             FROM latex_core.institution_import_jobs j \
             WHERE $2::uuid IS NULL OR (j.created_at,j.id) < (SELECT created_at,id FROM latex_core.institution_import_jobs WHERE id=$2) \
             ORDER BY j.created_at DESC,j.id DESC LIMIT $1",
        )
        .bind(limit)
        .bind(before)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        rows.into_iter().map(decode_job).collect()
    }

    pub async fn job(&self, id: Uuid) -> Result<InstitutionImportJob, InstitutionError> {
        let row = sqlx::query(
            "SELECT id,import_kind,mode,original_filename,content_sha256,file_type,submitted_by_user_id,status,total_rows,inserted_rows,updated_rows,skipped_rows,error_rows,created_at::text,validated_at::text,applied_at::text \
             FROM latex_core.institution_import_jobs WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?
        .ok_or(InstitutionError::NotFound)?;
        decode_job(row)
    }

    pub async fn job_rows(
        &self,
        id: Uuid,
        errors_only: bool,
    ) -> Result<Vec<InstitutionImportRow>, InstitutionError> {
        let rows = sqlx::query(
            "SELECT job_id,source_table_or_sheet,row_number,natural_key,payload,action,status,error_code,error_message \
             FROM latex_core.institution_import_rows WHERE job_id=$1 AND (NOT $2 OR status IN ('ERROR','UNRESOLVED')) \
             ORDER BY source_table_or_sheet,row_number",
        )
        .bind(id)
        .bind(errors_only)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        rows.into_iter().map(decode_import_row).collect()
    }

    pub async fn resolve_default_template_for_writers(
        &self,
        ordered_writer_user_ids: &[UserId],
    ) -> Result<TemplateResolution, InstitutionError> {
        let mut counts = BTreeMap::<String, usize>::new();
        let mut first = HashMap::<String, usize>::new();
        let mut warnings = Vec::new();
        for (position, user_id) in ordered_writer_user_ids.iter().enumerate() {
            let programme: Option<String> = sqlx::query_scalar(
                "SELECT student.programme_code FROM vcap.student_user_links link \
                 JOIN vcap.students student ON student.reg_no=link.reg_no \
                 WHERE link.user_id=$1 AND link.status='LINKED'",
            )
            .bind(user_id.as_uuid())
            .fetch_optional(self.database.pool())
            .await
            .map_err(InstitutionError::Database)?
            .flatten();
            if let Some(programme) = programme.filter(|value| !value.trim().is_empty()) {
                *counts.entry(programme.clone()).or_default() += 1;
                first.entry(programme).or_insert(position);
            } else {
                warnings.push(format!(
                    "writer {user_id} has no linked institutional programme"
                ));
            }
        }
        let maximum = counts.values().copied().max();
        let candidates = maximum.map_or_else(Vec::new, |maximum| {
            counts
                .iter()
                .filter_map(|(programme, count)| (*count == maximum).then_some(programme.clone()))
                .collect::<Vec<_>>()
        });
        let dominant = candidates
            .iter()
            .min_by_key(|programme| first.get(*programme).copied().unwrap_or(usize::MAX))
            .cloned();
        let mapped = if let Some(programme) = dominant.as_deref() {
            sqlx::query_scalar(
                "SELECT template_id FROM latex_core.programme_template_defaults WHERE programme_code=$1",
            )
            .bind(programme)
            .fetch_optional(self.database.pool())
            .await
            .map_err(InstitutionError::Database)?
        } else {
            None
        };
        let tie = candidates.len() > 1;
        let (selected_template_id, resolution_method) = if let Some(template) = mapped {
            (
                template,
                if tie { "TIE_FIRST_WRITER" } else { "MODE" }.to_owned(),
            )
        } else {
            if dominant.is_some() {
                warnings.push(
                    "dominant programme has no template mapping; using global fallback".into(),
                );
            } else {
                warnings.push("all Writers are unresolved; using global fallback".into());
            }
            let fallback: Option<Uuid> = sqlx::query_scalar(
                "SELECT global_fallback_template_id FROM latex_core.institution_template_config WHERE singleton",
            )
            .fetch_one(self.database.pool())
            .await
            .map_err(InstitutionError::Database)?;
            let template = fallback.ok_or_else(|| {
                InstitutionError::Conflict("global fallback template is not configured".into())
            })?;
            (template, "GLOBAL_FALLBACK".to_owned())
        };
        Ok(TemplateResolution {
            selected_template_id,
            dominant_programme_code: dominant,
            resolution_method,
            counts,
            tie_break: tie.then(|| "earliest ordered Writer programme".to_owned()),
            warnings,
        })
    }

    pub async fn list_programme_template_defaults(
        &self,
    ) -> Result<Vec<ProgrammeTemplateDefault>, InstitutionError> {
        let rows = sqlx::query(
            "SELECT p.programme_code,d.template_id,t.name AS template_name,d.updated_at::text \
             FROM vcap.programmes p LEFT JOIN latex_core.programme_template_defaults d USING(programme_code) \
             LEFT JOIN latex_core.templates t ON t.id=d.template_id ORDER BY p.programme_code",
        )
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        rows.into_iter()
            .map(|row| {
                Ok(ProgrammeTemplateDefault {
                    programme_code: row
                        .try_get("programme_code")
                        .map_err(InstitutionError::Database)?,
                    template_id: row
                        .try_get("template_id")
                        .map_err(InstitutionError::Database)?,
                    template_name: row
                        .try_get("template_name")
                        .map_err(InstitutionError::Database)?,
                    updated_at: row
                        .try_get("updated_at")
                        .map_err(InstitutionError::Database)?,
                })
            })
            .collect()
    }

    pub async fn set_programme_template_default(
        &self,
        actor: UserId,
        programme_code: &str,
        template_id: Uuid,
    ) -> Result<(), InstitutionError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        let result = sqlx::query(
            "INSERT INTO latex_core.programme_template_defaults (programme_code,template_id,updated_by_user_id) \
             VALUES ($1,$2,$3) ON CONFLICT(programme_code) DO UPDATE SET template_id=EXCLUDED.template_id,updated_by_user_id=EXCLUDED.updated_by_user_id,updated_at=statement_timestamp()",
        )
        .bind(programme_code)
        .bind(template_id)
        .bind(actor.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        if result.rows_affected() == 0 {
            return Err(InstitutionError::NotFound);
        }
        audit_tx(
            &mut tx,
            actor,
            "institution.programme_template.changed",
            "programme",
            Uuid::nil(),
            json!({"programme_code":programme_code,"template_id":template_id}),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)
    }

    pub async fn delete_programme_template_default(
        &self,
        actor: UserId,
        programme_code: &str,
    ) -> Result<(), InstitutionError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        sqlx::query("DELETE FROM latex_core.programme_template_defaults WHERE programme_code=$1")
            .bind(programme_code)
            .execute(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?;
        audit_tx(
            &mut tx,
            actor,
            "institution.programme_template.changed",
            "programme",
            Uuid::nil(),
            json!({"programme_code":programme_code,"template_id":null}),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)
    }

    pub async fn set_global_fallback(
        &self,
        actor: UserId,
        template_id: Uuid,
    ) -> Result<(), InstitutionError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        sqlx::query("UPDATE latex_core.institution_template_config SET global_fallback_template_id=$1,updated_by_user_id=$2,updated_at=statement_timestamp() WHERE singleton")
            .bind(template_id).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(InstitutionError::Database)?;
        audit_tx(
            &mut tx,
            actor,
            "institution.programme_template.changed",
            "global_fallback",
            Uuid::nil(),
            json!({"template_id":template_id}),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)
    }

    pub async fn global_fallback(&self) -> Result<Option<Uuid>, InstitutionError> {
        sqlx::query_scalar(
            "SELECT global_fallback_template_id FROM latex_core.institution_template_config WHERE singleton",
        )
        .fetch_one(self.database.pool())
        .await
        .map_err(InstitutionError::Database)
    }

    pub async fn mark_team_unresolved(
        &self,
        job_id: Uuid,
        external_team_key: &str,
        reasons: &[String],
    ) -> Result<(), InstitutionError> {
        let message = reasons.join("; ");
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        let affected = sqlx::query(
            "UPDATE latex_core.institution_import_rows SET status='UNRESOLVED',error_code='TEAM_UNRESOLVED',error_message=$3 \
             WHERE job_id=$1 AND (natural_key->>'external_team_key')=$2 AND status<>'UNRESOLVED'",
        )
        .bind(job_id)
        .bind(external_team_key)
        .bind(&message)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?
        .rows_affected();
        sqlx::query(
            "UPDATE latex_core.institution_import_jobs SET status='PARTIAL',error_rows=error_rows+$2 WHERE id=$1",
        )
        .bind(job_id)
        .bind(i64::try_from(affected).unwrap_or(i64::MAX))
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        tx.commit().await.map_err(InstitutionError::Database)
    }

    pub async fn pending_team_plans(
        &self,
        job_id: Uuid,
    ) -> Result<Vec<ImportedTeamPlan>, InstitutionError> {
        let groups = sqlx::query(
            "SELECT g.external_team_key,g.team_name,link.paper_team_id FROM vcap.paper_assignment_groups g \
             LEFT JOIN latex_core.external_paper_team_links link USING(external_team_key) \
             WHERE EXISTS (SELECT 1 FROM latex_core.institution_import_rows r WHERE r.job_id=$1 AND r.source_table_or_sheet IN ('paper_teams','paper_team_writers','paper_team_mentors') AND (r.natural_key->>'external_team_key')=g.external_team_key) \
             ORDER BY g.external_team_key",
        )
        .bind(job_id)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        let mut plans = Vec::with_capacity(groups.len());
        for group in groups {
            let key: String = group
                .try_get("external_team_key")
                .map_err(InstitutionError::Database)?;
            let team_name: String = group
                .try_get("team_name")
                .map_err(InstitutionError::Database)?;
            let existing_paper_team_id: Option<Uuid> = group
                .try_get("paper_team_id")
                .map_err(InstitutionError::Database)?;
            let writer_rows = sqlx::query(
                "SELECT assignment.student_reg_no,assignment.is_leader,link.user_id,link.status,role.role \
                 FROM vcap.paper_assignment_students assignment \
                 LEFT JOIN vcap.student_user_links link ON link.reg_no=assignment.student_reg_no \
                 LEFT JOIN latex_core.global_user_roles role ON role.user_id=link.user_id \
                 WHERE assignment.external_team_key=$1 ORDER BY assignment.writer_order",
            )
            .bind(&key)
            .fetch_all(self.database.pool())
            .await
            .map_err(InstitutionError::Database)?;
            let mentor_rows = sqlx::query(
                "SELECT assignment.faculty_id,link.user_id,link.status,role.role \
                 FROM vcap.paper_assignment_mentors assignment \
                 LEFT JOIN vcap.faculty_user_links link ON link.faculty_id=assignment.faculty_id \
                 LEFT JOIN latex_core.global_user_roles role ON role.user_id=link.user_id \
                 WHERE assignment.external_team_key=$1 ORDER BY assignment.faculty_id",
            )
            .bind(&key)
            .fetch_all(self.database.pool())
            .await
            .map_err(InstitutionError::Database)?;
            let mut writers = Vec::new();
            let mut leader = None;
            let mut mentors = Vec::new();
            let mut unresolved = Vec::new();
            for row in writer_rows {
                let reg_no: String = row
                    .try_get("student_reg_no")
                    .map_err(InstitutionError::Database)?;
                let user: Option<Uuid> =
                    row.try_get("user_id").map_err(InstitutionError::Database)?;
                let role: Option<String> =
                    row.try_get("role").map_err(InstitutionError::Database)?;
                let is_leader: bool = row
                    .try_get("is_leader")
                    .map_err(InstitutionError::Database)?;
                if let Some(user) = user.filter(|_| role.as_deref() == Some("writer")) {
                    writers.push(user);
                    if is_leader {
                        leader = Some(user);
                    }
                } else {
                    unresolved.push(format!("UNRESOLVED_WRITER:{reg_no}"));
                }
            }
            if leader.is_none() {
                unresolved.push("MISSING_LEADER".into());
            }
            if writers.is_empty() {
                unresolved.push("TEAM_HAS_NO_WRITER".into());
            }
            for row in mentor_rows {
                let faculty_id: String = row
                    .try_get("faculty_id")
                    .map_err(InstitutionError::Database)?;
                let user: Option<Uuid> =
                    row.try_get("user_id").map_err(InstitutionError::Database)?;
                let role: Option<String> =
                    row.try_get("role").map_err(InstitutionError::Database)?;
                if let Some(user) = user.filter(|_| role.as_deref() == Some("mentor")) {
                    mentors.push(user);
                } else {
                    unresolved.push(format!("UNRESOLVED_MENTOR:{faculty_id}"));
                }
            }
            plans.push(ImportedTeamPlan {
                external_team_key: key,
                team_name,
                existing_paper_team_id,
                writer_user_ids: writers,
                leader_user_id: leader,
                mentor_user_ids: mentors,
                unresolved,
            });
        }
        Ok(plans)
    }

    pub async fn merge_existing_team(
        &self,
        actor: UserId,
        plan: &ImportedTeamPlan,
    ) -> Result<(), InstitutionError> {
        let paper_team_id = plan.existing_paper_team_id.ok_or_else(|| {
            InstitutionError::InvalidInput("existing Paper Team link is missing".into())
        })?;
        let leader = plan
            .leader_user_id
            .ok_or_else(|| InstitutionError::Conflict("MISSING_LEADER".into()))?;
        if !plan.unresolved.is_empty() {
            return Err(InstitutionError::Conflict(plan.unresolved.join("; ")));
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        let admin_role: Option<String> =
            sqlx::query_scalar("SELECT role FROM latex_core.global_user_roles WHERE user_id=$1")
                .bind(actor.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(InstitutionError::Database)?;
        if admin_role.as_deref() != Some("admin") {
            return Err(InstitutionError::Conflict("V2 Admin required".into()));
        }
        sqlx::query("SELECT id FROM latex_core.paper_teams WHERE id=$1 FOR UPDATE")
            .bind(paper_team_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?
            .ok_or(InstitutionError::NotFound)?;
        let retained_writers: Vec<Uuid> = sqlx::query_scalar(
            "SELECT member.user_id FROM latex_core.paper_team_members member \
             JOIN latex_core.global_user_roles role ON role.user_id=member.user_id AND role.role='writer' \
             WHERE member.paper_team_id=$1 AND NOT(member.user_id=ANY($2)) \
             ORDER BY member.writer_order NULLS LAST,member.created_at,member.user_id",
        )
        .bind(paper_team_id)
        .bind(&plan.writer_user_ids)
        .fetch_all(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        sqlx::query(
            "UPDATE latex_core.paper_team_members member SET writer_order=NULL,is_leader=FALSE \
             FROM latex_core.global_user_roles role WHERE member.paper_team_id=$1 AND role.user_id=member.user_id AND role.role='writer'",
        )
        .bind(paper_team_id)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        for (position, writer) in plan.writer_user_ids.iter().enumerate() {
            let writer_order = i32::try_from(position + 1)
                .map_err(|_| InstitutionError::InvalidInput("too many Team Writers".into()))?;
            sqlx::query(
                "INSERT INTO latex_core.paper_team_members (paper_team_id,user_id,assigned_by_user_id,is_leader,writer_order) \
                 VALUES ($1,$2,$3,$4,$5) ON CONFLICT(paper_team_id,user_id) DO UPDATE \
                 SET is_leader=EXCLUDED.is_leader,writer_order=EXCLUDED.writer_order",
            )
            .bind(paper_team_id)
            .bind(writer)
            .bind(actor.as_uuid())
            .bind(*writer == leader)
            .bind(writer_order)
            .execute(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?;
        }
        for (offset, writer) in retained_writers.iter().enumerate() {
            let order = plan
                .writer_user_ids
                .len()
                .checked_add(offset + 1)
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| InstitutionError::InvalidInput("too many Team Writers".into()))?;
            sqlx::query(
                "UPDATE latex_core.paper_team_members SET writer_order=$3 WHERE paper_team_id=$1 AND user_id=$2",
            )
            .bind(paper_team_id)
            .bind(writer)
            .bind(order)
            .execute(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?;
        }
        for mentor in &plan.mentor_user_ids {
            sqlx::query(
                "INSERT INTO latex_core.paper_team_members (paper_team_id,user_id,assigned_by_user_id) \
                 VALUES ($1,$2,$3) ON CONFLICT(paper_team_id,user_id) DO NOTHING",
            )
            .bind(paper_team_id)
            .bind(mentor)
            .bind(actor.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?;
        }
        sqlx::query(
            "UPDATE latex_core.paper_teams SET name=$2,updated_at=statement_timestamp() WHERE id=$1",
        )
        .bind(paper_team_id)
        .bind(plan.team_name.trim())
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        audit_tx(
            &mut tx,
            actor,
            "institution.team.merged",
            "paper_team",
            paper_team_id,
            json!({"external_team_key":plan.external_team_key,"writers_added_or_ordered":plan.writer_user_ids.len(),"mentors_added":plan.mentor_user_ids.len()}),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)
    }

    pub async fn paginated_paper_teams(
        &self,
        filter: &PaperTeamPageFilter,
    ) -> Result<PaperTeamPage, InstitutionError> {
        let limit = if filter.limit == 0 {
            50
        } else {
            filter.limit.clamp(1, 200)
        };
        let page = filter.page.max(1);
        let offset = (page - 1).saturating_mul(limit);
        if filter.unresolved == Some(true) {
            return self
                .paginated_unresolved_assignments(filter, page, limit, offset)
                .await;
        }
        let rows = sqlx::query(
            r"SELECT t.id,t.name,t.status,t.updated_at::text AS updated_at,
                     count(*) OVER() AS total,
                     (SELECT count(*) FROM latex_core.paper_team_members m JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role='writer' WHERE m.paper_team_id=t.id) AS writer_count,
                     (SELECT jsonb_build_object('user_id',m.user_id,'email',c.email) FROM latex_core.paper_team_members m JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role='writer' JOIN latex_core.user_credentials c ON c.user_id=m.user_id WHERE m.paper_team_id=t.id AND m.is_leader LIMIT 1) AS leader,
                     COALESCE((SELECT jsonb_agg(jsonb_build_object('user_id',m.user_id,'email',c.email) ORDER BY c.email) FROM latex_core.paper_team_members m JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role='mentor' JOIN latex_core.user_credentials c ON c.user_id=m.user_id WHERE m.paper_team_id=t.id),'[]'::jsonb) AS mentors,
                     resolution.dominant_programme_code,resolution.resolution_method,
                     pin.template_id,template.name AS template_name,
                     COALESCE(review.status,'NONE') AS review_state,
                     link.external_team_key,CASE WHEN link.external_team_key IS NULL THEN 'manual' ELSE 'imported' END AS source
              FROM latex_core.paper_teams t
              LEFT JOIN latex_core.paper_template_resolutions resolution ON resolution.paper_team_id=t.id
              LEFT JOIN latex_core.paper_template_pins pin ON pin.paper_id=t.id
              LEFT JOIN latex_core.templates template ON template.id=pin.template_id
              LEFT JOIN latex_core.external_paper_team_links link ON link.paper_team_id=t.id
              LEFT JOIN LATERAL (SELECT r.status FROM latex_core.review_rounds r WHERE r.paper_id=t.id ORDER BY r.opened_at DESC,r.id DESC LIMIT 1) review ON TRUE
              WHERE ($1::text IS NULL OR t.name ILIKE '%' || $1 || '%' OR link.external_team_key ILIKE '%' || $1 || '%')
                AND ($2::text IS NULL OR t.status=$2)
                AND ($3::text IS NULL OR resolution.dominant_programme_code=$3)
                AND ($4::uuid IS NULL OR EXISTS (SELECT 1 FROM latex_core.paper_team_members m JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role='mentor' WHERE m.paper_team_id=t.id AND m.user_id=$4))
                AND ($5::uuid IS NULL OR EXISTS (SELECT 1 FROM latex_core.paper_team_members m WHERE m.paper_team_id=t.id AND m.user_id=$5 AND m.is_leader))
                AND ($6::uuid IS NULL OR pin.template_id=$6)
                AND ($7::text IS NULL OR COALESCE(review.status,'NONE')=$7)
                AND ($8::text IS NULL OR ($8='manual' AND link.external_team_key IS NULL) OR ($8='imported' AND link.external_team_key IS NOT NULL))
                AND ($9::boolean IS NULL OR $9=FALSE)
              ORDER BY t.updated_at DESC,t.id DESC LIMIT $10 OFFSET $11",
        )
        .bind(
            filter
                .search
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        )
        .bind(filter.status.as_deref())
        .bind(filter.programme_code.as_deref())
        .bind(filter.mentor_user_id)
        .bind(filter.leader_user_id)
        .bind(filter.template_id)
        .bind(filter.review_state.as_deref())
        .bind(filter.source.as_deref())
        .bind(filter.unresolved)
        .bind(limit)
        .bind(offset)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        let total = rows
            .first()
            .map_or(Ok(0_i64), |row| row.try_get("total"))
            .map_err(InstitutionError::Database)?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            items.push(json!({
                "id": row.try_get::<Uuid,_>("id").map_err(InstitutionError::Database)?,
                "name": row.try_get::<String,_>("name").map_err(InstitutionError::Database)?,
                "status": row.try_get::<String,_>("status").map_err(InstitutionError::Database)?,
                "writer_count": row.try_get::<i64,_>("writer_count").map_err(InstitutionError::Database)?,
                "leader": row.try_get::<Option<Value>,_>("leader").map_err(InstitutionError::Database)?,
                "mentors": row.try_get::<Value,_>("mentors").map_err(InstitutionError::Database)?,
                "dominant_programme_code": row.try_get::<Option<String>,_>("dominant_programme_code").map_err(InstitutionError::Database)?,
                "template": {"id":row.try_get::<Option<Uuid>,_>("template_id").map_err(InstitutionError::Database)?,"name":row.try_get::<Option<String>,_>("template_name").map_err(InstitutionError::Database)?},
                "template_resolution_method": row.try_get::<Option<String>,_>("resolution_method").map_err(InstitutionError::Database)?,
                "review_state": row.try_get::<String,_>("review_state").map_err(InstitutionError::Database)?,
                "updated_at": row.try_get::<String,_>("updated_at").map_err(InstitutionError::Database)?,
                "source": row.try_get::<String,_>("source").map_err(InstitutionError::Database)?,
                "external_team_key": row.try_get::<Option<String>,_>("external_team_key").map_err(InstitutionError::Database)?,
                "unresolved": false,
            }));
        }
        let returned = i64::try_from(items.len()).unwrap_or(i64::MAX);
        Ok(PaperTeamPage {
            total,
            has_more: offset.saturating_add(returned) < total,
            page,
            limit,
            items,
        })
    }

    async fn paginated_unresolved_assignments(
        &self,
        filter: &PaperTeamPageFilter,
        page: i64,
        limit: i64,
        offset: i64,
    ) -> Result<PaperTeamPage, InstitutionError> {
        if filter.source.as_deref() == Some("manual")
            || filter.template_id.is_some()
            || filter.review_state.is_some()
        {
            return Ok(PaperTeamPage {
                total: 0,
                has_more: false,
                page,
                limit,
                items: Vec::new(),
            });
        }
        let rows = sqlx::query(
            r"WITH unresolved AS (
                  SELECT g.external_team_key,g.team_name,g.status,
                         max(job.created_at)::text AS updated_at,
                         count(DISTINCT writer.student_reg_no) AS writer_count,
                         count(*) OVER() AS total
                  FROM vcap.paper_assignment_groups g
                  JOIN latex_core.institution_import_rows import_row ON (import_row.natural_key->>'external_team_key')=g.external_team_key AND import_row.status='UNRESOLVED'
                  JOIN latex_core.institution_import_jobs job ON job.id=import_row.job_id
                  LEFT JOIN vcap.paper_assignment_students writer ON writer.external_team_key=g.external_team_key
                  LEFT JOIN latex_core.external_paper_team_links link ON link.external_team_key=g.external_team_key
                  WHERE link.external_team_key IS NULL
                    AND ($1::text IS NULL OR g.team_name ILIKE '%' || $1 || '%' OR g.external_team_key ILIKE '%' || $1 || '%')
                    AND ($2::text IS NULL OR COALESCE(g.status,'unresolved')=$2)
                    AND ($3::text IS NULL OR EXISTS (SELECT 1 FROM vcap.paper_assignment_students assigned JOIN vcap.students student ON student.reg_no=assigned.student_reg_no WHERE assigned.external_team_key=g.external_team_key AND student.programme_code=$3))
                    AND ($4::uuid IS NULL OR EXISTS (SELECT 1 FROM vcap.paper_assignment_mentors assigned JOIN vcap.faculty_user_links identity ON identity.faculty_id=assigned.faculty_id AND identity.status='LINKED' WHERE assigned.external_team_key=g.external_team_key AND identity.user_id=$4))
                    AND ($5::uuid IS NULL OR EXISTS (SELECT 1 FROM vcap.paper_assignment_students assigned JOIN vcap.student_user_links identity ON identity.reg_no=assigned.student_reg_no AND identity.status='LINKED' WHERE assigned.external_team_key=g.external_team_key AND assigned.is_leader AND identity.user_id=$5))
                  GROUP BY g.external_team_key,g.team_name,g.status
              ) SELECT * FROM unresolved ORDER BY updated_at DESC,external_team_key LIMIT $6 OFFSET $7",
        )
        .bind(
            filter
                .search
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        )
        .bind(filter.status.as_deref())
        .bind(filter.programme_code.as_deref())
        .bind(filter.mentor_user_id)
        .bind(filter.leader_user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        let total = rows
            .first()
            .map_or(Ok(0_i64), |row| row.try_get("total"))
            .map_err(InstitutionError::Database)?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            items.push(json!({
                "id": null,
                "name": row.try_get::<String,_>("team_name").map_err(InstitutionError::Database)?,
                "status": row.try_get::<Option<String>,_>("status").map_err(InstitutionError::Database)?.unwrap_or_else(|| "unresolved".into()),
                "writer_count": row.try_get::<i64,_>("writer_count").map_err(InstitutionError::Database)?,
                "leader": null,
                "mentors": [],
                "dominant_programme_code": null,
                "template": {"id":null,"name":null},
                "template_resolution_method": null,
                "review_state": "NONE",
                "updated_at": row.try_get::<String,_>("updated_at").map_err(InstitutionError::Database)?,
                "source": "imported",
                "external_team_key": row.try_get::<String,_>("external_team_key").map_err(InstitutionError::Database)?,
                "unresolved": true,
            }));
        }
        let returned = i64::try_from(items.len()).unwrap_or(i64::MAX);
        Ok(PaperTeamPage {
            total,
            has_more: offset.saturating_add(returned) < total,
            page,
            limit,
            items,
        })
    }

    async fn assign_actions(
        &self,
        rows: &mut [SourceRow],
        mode: ImportMode,
    ) -> Result<(), InstitutionError> {
        let mut existing = HashMap::<String, HashSet<String>>::new();
        for table in INSTITUTION_TABLES {
            if rows
                .iter()
                .any(|row| row.table == table && row.error.is_none())
            {
                existing.insert(table.to_owned(), self.existing_keys(table).await?);
            }
        }
        for row in rows {
            if row.error.is_some() {
                continue;
            }
            let present = existing
                .get(&row.table)
                .is_some_and(|keys| keys.contains(&key_token(&row.natural_key)));
            row.payload
                .as_object_mut()
                .expect("validated payload object")
                .insert(
                    "__action".into(),
                    Value::String(
                        match (present, mode) {
                            (false, _) => "INSERT",
                            (true, ImportMode::Merge) => "UPDATE",
                            (true, ImportMode::ValidateOnly | ImportMode::AddOnly) => "SKIP",
                        }
                        .into(),
                    ),
                );
        }
        Ok(())
    }

    async fn existing_keys(&self, table: &str) -> Result<HashSet<String>, InstitutionError> {
        let query = match table {
            "departments" => {
                "SELECT jsonb_build_object('department_id',department_id::text) FROM vcap.departments"
            }
            "admins" => "SELECT jsonb_build_object('admin_id',admin_id) FROM vcap.admins",
            "faculty" => "SELECT jsonb_build_object('faculty_id',faculty_id) FROM vcap.faculty",
            "programmes" => {
                "SELECT jsonb_build_object('programme_code',programme_code) FROM vcap.programmes"
            }
            "schools" => "SELECT jsonb_build_object('school_id',school_id) FROM vcap.schools",
            "students" => "SELECT jsonb_build_object('reg_no',reg_no) FROM vcap.students",
            "student_course_registrations" => {
                "SELECT jsonb_build_object('student_reg_no',student_reg_no,'course_id',course_id,'academic_year',academic_year,'semester',semester) FROM vcap.student_course_registrations"
            }
            "faculty_guide_capacity" => {
                "SELECT jsonb_build_object('capacity_id',capacity_id::text) FROM vcap.faculty_guide_capacity"
            }
            "department_roles" => {
                "SELECT jsonb_build_object('id',id::text) FROM vcap.department_roles"
            }
            "faculty_roles" => {
                "SELECT jsonb_build_object('role_id',role_id::text) FROM vcap.faculty_roles"
            }
            "paper_teams" => {
                "SELECT jsonb_build_object('external_team_key',external_team_key) FROM vcap.paper_assignment_groups"
            }
            "paper_team_writers" => {
                "SELECT jsonb_build_object('external_team_key',external_team_key,'student_reg_no',student_reg_no) FROM vcap.paper_assignment_students"
            }
            "paper_team_mentors" => {
                "SELECT jsonb_build_object('external_team_key',external_team_key,'faculty_id',faculty_id) FROM vcap.paper_assignment_mentors"
            }
            _ => {
                return Err(InstitutionError::InvalidInput(
                    "unknown target table".into(),
                ));
            }
        };
        let values: Vec<Value> = sqlx::query_scalar(query)
            .fetch_all(self.database.pool())
            .await
            .map_err(InstitutionError::Database)?;
        Ok(values.into_iter().map(|value| key_token(&value)).collect())
    }

    async fn validate_database_references(
        &self,
        rows: &mut [SourceRow],
    ) -> Result<(), InstitutionError> {
        let departments: HashSet<String> =
            sqlx::query_scalar::<_, Uuid>("SELECT department_id FROM vcap.departments")
                .fetch_all(self.database.pool())
                .await
                .map_err(InstitutionError::Database)?
                .into_iter()
                .map(|id| id.to_string())
                .collect();
        let programmes: HashSet<String> =
            sqlx::query_scalar("SELECT programme_code FROM vcap.programmes")
                .fetch_all(self.database.pool())
                .await
                .map_err(InstitutionError::Database)?
                .into_iter()
                .collect();
        let schools: HashSet<String> = sqlx::query_scalar("SELECT school_id FROM vcap.schools")
            .fetch_all(self.database.pool())
            .await
            .map_err(InstitutionError::Database)?
            .into_iter()
            .collect();
        let faculty: HashSet<String> = sqlx::query_scalar("SELECT faculty_id FROM vcap.faculty")
            .fetch_all(self.database.pool())
            .await
            .map_err(InstitutionError::Database)?
            .into_iter()
            .collect();
        let students: HashSet<String> = sqlx::query_scalar("SELECT reg_no FROM vcap.students")
            .fetch_all(self.database.pool())
            .await
            .map_err(InstitutionError::Database)?
            .into_iter()
            .collect();
        let teams: HashSet<String> =
            sqlx::query_scalar("SELECT external_team_key FROM vcap.paper_assignment_groups")
                .fetch_all(self.database.pool())
                .await
                .map_err(InstitutionError::Database)?
                .into_iter()
                .collect();
        let uploaded = uploaded_key_sets(rows);
        for row in rows {
            if row.error.is_some() {
                continue;
            }
            let payload = row.payload.as_object().expect("validated payload object");
            let check = |field: &str, db: &HashSet<String>, source: &str| -> Option<String> {
                text_value(payload, field)
                    .filter(|value| !value.is_empty())
                    .and_then(|value| {
                        (!db.contains(value)
                            && !uploaded.get(source).is_some_and(|set| set.contains(value)))
                        .then(|| format!("{field} references missing {source} value {value}"))
                    })
            };
            let error = match row.table.as_str() {
                "faculty" => check("dept_id", &departments, "departments"),
                "programmes" => check("hod_id", &faculty, "faculty"),
                "students" => check("programme_code", &programmes, "programmes"),
                "student_course_registrations" => check("student_reg_no", &students, "students"),
                "faculty_guide_capacity" => check("faculty_id", &faculty, "faculty"),
                "department_roles" => check("dept_id", &departments, "departments")
                    .or_else(|| check("faculty_id", &faculty, "faculty")),
                "faculty_roles" => check("faculty_id", &faculty, "faculty")
                    .or_else(|| check("school_id", &schools, "schools"))
                    .or_else(|| check("department_id", &departments, "departments"))
                    .or_else(|| check("programme_code", &programmes, "programmes")),
                "paper_team_writers" => check("external_team_key", &teams, "paper_teams")
                    .or_else(|| check("student_reg_no", &students, "students")),
                "paper_team_mentors" => check("external_team_key", &teams, "paper_teams")
                    .or_else(|| check("faculty_id", &faculty, "faculty")),
                _ => None,
            };
            if let Some(message) = error {
                row.error = Some(("INVALID_FK", message));
            }
        }
        Ok(())
    }
}

fn file_type(filename: &str) -> Result<ImportFileType, InstitutionError> {
    match filename
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("csv") => Ok(ImportFileType::Csv),
        Some("xlsx") => Ok(ImportFileType::Xlsx),
        _ => Err(InstitutionError::InvalidInput(
            "only CSV and XLSX are supported".into(),
        )),
    }
}

fn infer_csv_table(filename: &str) -> Option<&str> {
    let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
    INSTITUTION_TABLES.into_iter().find(|table| *table == stem)
}

fn parse_csv(
    filename: &str,
    target: Option<&str>,
    bytes: &[u8],
    limits: ImportLimits,
) -> Result<Vec<SourceRow>, InstitutionError> {
    let table = target
        .or_else(|| infer_csv_table(filename))
        .ok_or_else(|| {
            InstitutionError::InvalidInput(
                "CSV target table is required or filename must exactly match a supported table"
                    .into(),
            )
        })?;
    if !INSTITUTION_TABLES.contains(&table) {
        return Err(InstitutionError::InvalidInput(
            "unknown CSV target table".into(),
        ));
    }
    let mut reader = csv::ReaderBuilder::new().flexible(false).from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|error| InstitutionError::InvalidInput(format!("malformed CSV header: {error}")))?
        .clone();
    validate_headers(table, headers.iter(), limits)?;
    let mut rows = Vec::new();
    for (index, result) in reader.records().enumerate() {
        if index >= limits.max_rows_per_sheet {
            return Err(InstitutionError::InvalidInput(
                "CSV row limit exceeded".into(),
            ));
        }
        let record = result.map_err(|error| {
            InstitutionError::InvalidInput(format!("malformed CSV row: {error}"))
        })?;
        rows.push(source_row(
            table,
            i64::try_from(index + 2)
                .map_err(|_| InstitutionError::InvalidInput("row number overflow".into()))?,
            &headers,
            &record,
            None,
            limits,
        )?);
    }
    Ok(rows)
}

fn parse_xlsx(bytes: &[u8], limits: ImportLimits) -> Result<Vec<SourceRow>, InstitutionError> {
    let mut workbook = Xlsx::new(Cursor::new(bytes)).map_err(|error| {
        InstitutionError::InvalidInput(format!("malformed XLSX workbook: {error}"))
    })?;
    let names = workbook.sheet_names().clone();
    if names.len() > limits.max_worksheets {
        return Err(InstitutionError::InvalidInput(
            "worksheet limit exceeded".into(),
        ));
    }
    let mut output = Vec::new();
    for name in names {
        let range = workbook.worksheet_range(&name).map_err(|error| {
            InstitutionError::InvalidInput(format!("malformed worksheet {name}: {error}"))
        })?;
        if range.is_empty() {
            continue;
        }
        if !INSTITUTION_TABLES.contains(&name.as_str()) {
            return Err(InstitutionError::InvalidInput(format!(
                "unknown non-empty worksheet: {name}"
            )));
        }
        let (height, width) = range.get_size();
        if height.saturating_sub(1) > limits.max_rows_per_sheet || width > limits.max_columns {
            return Err(InstitutionError::InvalidInput(format!(
                "worksheet {name} dimensions exceed configured limits"
            )));
        }
        let formula_range = workbook.worksheet_formula(&name).map_err(|error| {
            InstitutionError::InvalidInput(format!("invalid formulas in worksheet {name}: {error}"))
        })?;
        let mut iter = range.rows();
        let header_cells = iter.next().ok_or_else(|| {
            InstitutionError::InvalidInput(format!("worksheet {name} has no header"))
        })?;
        let header_values = header_cells
            .iter()
            .map(cell_string)
            .collect::<Result<Vec<_>, _>>()?;
        validate_headers(&name, header_values.iter().map(String::as_str), limits)?;
        let headers = StringRecord::from(header_values);
        let key_indexes: HashSet<usize> = spec(&name)
            .keys
            .iter()
            .filter_map(|key| headers.iter().position(|header| header == *key))
            .collect();
        for (index, cells) in iter.enumerate() {
            let values = cells
                .iter()
                .map(cell_string)
                .collect::<Result<Vec<_>, _>>()?;
            let record = StringRecord::from(values);
            let formula_key = key_indexes.iter().any(|column| {
                formula_range
                    .get_value((
                        u32::try_from(index + 1).unwrap_or(u32::MAX),
                        u32::try_from(*column).unwrap_or(u32::MAX),
                    ))
                    .is_some_and(|formula| !formula.is_empty())
            });
            output.push(source_row(
                &name,
                i64::try_from(index + 2)
                    .map_err(|_| InstitutionError::InvalidInput("row number overflow".into()))?,
                &headers,
                &record,
                formula_key.then_some("formula in identity/key cell"),
                limits,
            )?);
        }
    }
    if output.is_empty() {
        return Err(InstitutionError::InvalidInput(
            "workbook has no recognized data rows".into(),
        ));
    }
    Ok(output)
}

fn cell_string(cell: &Data) -> Result<String, InstitutionError> {
    Ok(match cell {
        Data::Empty => String::new(),
        Data::String(value) | Data::DateTimeIso(value) | Data::DurationIso(value) => value.clone(),
        Data::Float(value) => {
            if value.fract() == 0.0 {
                format!("{value:.0}")
            } else {
                value.to_string()
            }
        }
        Data::Int(value) => value.to_string(),
        Data::Bool(value) => value.to_string(),
        Data::Error(error) => {
            return Err(InstitutionError::InvalidInput(format!(
                "XLSX cell error: {error:?}"
            )));
        }
        Data::DateTime(value) => value.to_string(),
    })
}

#[derive(Clone, Copy)]
struct TableSpec {
    columns: &'static [&'static str],
    required: &'static [&'static str],
    keys: &'static [&'static str],
    uuids: &'static [&'static str],
    integers: &'static [&'static str],
    emails: &'static [&'static str],
}

fn spec(table: &str) -> TableSpec {
    match table {
        "departments" => TableSpec {
            columns: &["department_id"],
            required: &["department_id"],
            keys: &["department_id"],
            uuids: &["department_id"],
            integers: &[],
            emails: &[],
        },
        "admins" => TableSpec {
            columns: &["admin_id", "email", "name", "pfp"],
            required: &["admin_id"],
            keys: &["admin_id"],
            uuids: &[],
            integers: &[],
            emails: &["email"],
        },
        "faculty" => TableSpec {
            columns: &[
                "faculty_id",
                "name",
                "email",
                "dept_id",
                "honorific",
                "designation",
                "status",
            ],
            required: &["faculty_id"],
            keys: &["faculty_id"],
            uuids: &["dept_id"],
            integers: &[],
            emails: &["email"],
        },
        "programmes" => TableSpec {
            columns: &["programme_code", "hod_id"],
            required: &["programme_code"],
            keys: &["programme_code"],
            uuids: &[],
            integers: &[],
            emails: &[],
        },
        "schools" => TableSpec {
            columns: &["school_id"],
            required: &["school_id"],
            keys: &["school_id"],
            uuids: &[],
            integers: &[],
            emails: &[],
        },
        "students" => TableSpec {
            columns: &["reg_no", "name", "email", "programme_code"],
            required: &["reg_no"],
            keys: &["reg_no"],
            uuids: &[],
            integers: &[],
            emails: &["email"],
        },
        "student_course_registrations" => TableSpec {
            columns: &[
                "student_reg_no",
                "course_id",
                "academic_year",
                "semester",
                "registration_status",
            ],
            required: &["student_reg_no", "course_id", "academic_year", "semester"],
            keys: &["student_reg_no", "course_id", "academic_year", "semester"],
            uuids: &[],
            integers: &[],
            emails: &[],
        },
        "faculty_guide_capacity" => TableSpec {
            columns: &[
                "capacity_id",
                "faculty_id",
                "academic_year",
                "ug_max_projects",
                "pg_max_projects",
                "integrated_pg_max_projects",
                "status",
            ],
            required: &["capacity_id"],
            keys: &["capacity_id"],
            uuids: &["capacity_id"],
            integers: &[
                "ug_max_projects",
                "pg_max_projects",
                "integrated_pg_max_projects",
            ],
            emails: &[],
        },
        "department_roles" => TableSpec {
            columns: &["id", "dept_id", "role_type", "faculty_id"],
            required: &["id", "dept_id"],
            keys: &["id"],
            uuids: &["dept_id"],
            integers: &["id"],
            emails: &[],
        },
        "faculty_roles" => TableSpec {
            columns: &[
                "role_id",
                "faculty_id",
                "role_type",
                "school_id",
                "department_id",
                "programme_code",
                "status",
            ],
            required: &["role_id"],
            keys: &["role_id"],
            uuids: &["role_id", "department_id"],
            integers: &[],
            emails: &[],
        },
        "paper_teams" => TableSpec {
            columns: &[
                "external_team_key",
                "team_name",
                "academic_year",
                "semester",
                "status",
            ],
            required: &["external_team_key", "team_name"],
            keys: &["external_team_key"],
            uuids: &[],
            integers: &[],
            emails: &[],
        },
        "paper_team_writers" => TableSpec {
            columns: &[
                "external_team_key",
                "student_reg_no",
                "writer_order",
                "is_leader",
            ],
            required: &[
                "external_team_key",
                "student_reg_no",
                "writer_order",
                "is_leader",
            ],
            keys: &["external_team_key", "student_reg_no"],
            uuids: &[],
            integers: &["writer_order"],
            emails: &[],
        },
        "paper_team_mentors" => TableSpec {
            columns: &["external_team_key", "faculty_id"],
            required: &["external_team_key", "faculty_id"],
            keys: &["external_team_key", "faculty_id"],
            uuids: &[],
            integers: &[],
            emails: &[],
        },
        _ => unreachable!("table is checked before spec lookup"),
    }
}

fn validate_headers<'a>(
    table: &str,
    headers: impl Iterator<Item = &'a str>,
    limits: ImportLimits,
) -> Result<(), InstitutionError> {
    let values = headers.map(str::trim).collect::<Vec<_>>();
    if values.is_empty() || values.len() > limits.max_columns {
        return Err(InstitutionError::InvalidInput(format!(
            "{table}: invalid column count"
        )));
    }
    let mut unique = HashSet::new();
    for value in &values {
        if value.is_empty() || !unique.insert(*value) {
            return Err(InstitutionError::InvalidInput(format!(
                "{table}: empty or duplicate column {value}"
            )));
        }
        if !spec(table).columns.contains(value) {
            return Err(InstitutionError::InvalidInput(format!(
                "{table}: unknown column {value}"
            )));
        }
    }
    for required in spec(table).required {
        if !unique.contains(required) {
            return Err(InstitutionError::InvalidInput(format!(
                "{table}: missing required column {required}"
            )));
        }
    }
    Ok(())
}

fn source_row(
    table: &str,
    row_number: i64,
    headers: &StringRecord,
    record: &StringRecord,
    formula_error: Option<&str>,
    limits: ImportLimits,
) -> Result<SourceRow, InstitutionError> {
    let table_spec = spec(table);
    let mut payload = Map::new();
    for (header, value) in headers.iter().zip(record.iter()) {
        let value = value.trim();
        if value.chars().count() > limits.max_cell_characters {
            return Err(InstitutionError::InvalidInput(format!(
                "{table} row {row_number}: cell character limit exceeded"
            )));
        }
        payload.insert(
            header.to_owned(),
            if value.is_empty() {
                Value::Null
            } else {
                Value::String(value.to_owned())
            },
        );
    }
    let mut error = formula_error.map(|message| ("FORMULA_IN_KEY", message.to_owned()));
    if error.is_none() {
        for field in table_spec.required {
            if text_value(&payload, field).is_none_or(str::is_empty) {
                error = Some(("MISSING_VALUE", format!("required value {field} is empty")));
                break;
            }
        }
    }
    if error.is_none() {
        for field in table_spec.uuids {
            if let Some(value) = text_value(&payload, field).filter(|value| !value.is_empty()) {
                if Uuid::parse_str(value).is_err() {
                    error = Some(("MALFORMED_UUID", format!("{field} is not a UUID")));
                    break;
                }
            }
        }
    }
    if error.is_none() {
        for field in table_spec.integers {
            if let Some(value) = text_value(&payload, field).filter(|value| !value.is_empty()) {
                if value.parse::<i32>().ok().is_none_or(|number| {
                    number < 0
                        || (*field == "writer_order" && number == 0)
                        || (*field == "id" && number == 0)
                }) {
                    error = Some((
                        "INVALID_INTEGER",
                        format!("{field} is outside its valid range"),
                    ));
                    break;
                }
            }
        }
    }
    if error.is_none() {
        for field in table_spec.emails {
            if let Some(value) = text_value(&payload, field).filter(|value| !value.is_empty()) {
                if !valid_email(value) {
                    error = Some((
                        "INVALID_EMAIL",
                        format!("{field} is not a normalized email address"),
                    ));
                    break;
                }
                payload.insert(
                    (*field).to_owned(),
                    Value::String(value.to_ascii_lowercase()),
                );
            }
        }
    }
    if error.is_none() && table == "paper_team_writers" {
        let leader = text_value(&payload, "is_leader")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(leader.as_str(), "true" | "false" | "1" | "0" | "yes" | "no") {
            error = Some(("INVALID_BOOLEAN", "is_leader must be true or false".into()));
        }
        payload.insert(
            "is_leader".into(),
            Value::Bool(matches!(leader.as_str(), "true" | "1" | "yes")),
        );
    }
    let natural_key = Value::Object(
        table_spec
            .keys
            .iter()
            .map(|key| {
                (
                    (*key).to_owned(),
                    payload.get(*key).cloned().unwrap_or(Value::Null),
                )
            })
            .collect(),
    );
    Ok(SourceRow {
        table: table.to_owned(),
        row_number,
        natural_key,
        payload: Value::Object(payload),
        error,
    })
}

fn validate_cross_rows(rows: &mut [SourceRow]) {
    let mut keys = HashSet::new();
    for row in rows.iter_mut() {
        if row.error.is_none() && !keys.insert((row.table.clone(), key_token(&row.natural_key))) {
            row.error = Some((
                "DUPLICATE_KEY",
                "duplicate primary key within import source".into(),
            ));
        }
    }
    let mut writer_orders = HashSet::new();
    let mut leaders = HashMap::<String, usize>::new();
    let mut writers = HashMap::<String, usize>::new();
    let includes_writer_sheet = rows
        .iter()
        .any(|candidate| candidate.table == "paper_team_writers");
    for row in rows
        .iter_mut()
        .filter(|row| row.table == "paper_team_writers" && row.error.is_none())
    {
        let payload = row.payload.as_object().expect("payload object");
        let team = text_value(payload, "external_team_key")
            .unwrap_or_default()
            .to_owned();
        let order = text_value(payload, "writer_order")
            .unwrap_or_default()
            .to_owned();
        if !writer_orders.insert((team.clone(), order)) {
            row.error = Some((
                "DUPLICATE_WRITER_ORDER",
                "writer_order must be unique per Team".into(),
            ));
            continue;
        }
        *writers.entry(team.clone()).or_default() += 1;
        if payload.get("is_leader").and_then(Value::as_bool) == Some(true) {
            *leaders.entry(team).or_default() += 1;
        }
    }
    for row in rows
        .iter_mut()
        .filter(|row| row.table == "paper_teams" && row.error.is_none())
    {
        let team = text_value(
            row.payload.as_object().expect("payload object"),
            "external_team_key",
        )
        .unwrap_or_default();
        if includes_writer_sheet && writers.get(team).copied().unwrap_or(0) == 0 {
            row.error = Some(("TEAM_HAS_NO_WRITER", "Team has no Writer row".into()));
        }
        if leaders.get(team).copied().unwrap_or(0) > 1 {
            row.error = Some((
                "MULTIPLE_LEADERS",
                "Team has more than one imported Leader".into(),
            ));
        }
    }
}

fn uploaded_key_sets(rows: &[SourceRow]) -> HashMap<String, HashSet<String>> {
    let mut sets = HashMap::<String, HashSet<String>>::new();
    for row in rows.iter().filter(|row| row.error.is_none()) {
        for value in row
            .natural_key
            .as_object()
            .into_iter()
            .flat_map(|map| map.values())
            .filter_map(Value::as_str)
        {
            sets.entry(row.table.clone())
                .or_default()
                .insert(value.to_owned());
        }
    }
    sets
}

fn action_for(row: &SourceRow) -> &str {
    row.payload
        .get("__action")
        .and_then(Value::as_str)
        .unwrap_or("INSERT")
}

fn key_token(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn text_value<'a>(map: &'a Map<String, Value>, field: &str) -> Option<&'a str> {
    map.get(field).and_then(Value::as_str)
}

fn valid_email(value: &str) -> bool {
    value.len() <= 320
        && value == value.trim()
        && !value.chars().any(char::is_whitespace)
        && value.split('@').count() == 2
        && value.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        })
}

async fn apply_table(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    table: &str,
    merge: bool,
    payloads: Value,
) -> Result<(), InstitutionError> {
    let query = apply_query(table, merge);
    sqlx::query(query)
        .bind(payloads)
        .execute(&mut **tx)
        .await
        .map_err(InstitutionError::Database)?;
    if table == "department_roles" {
        sqlx::query("SELECT setval(pg_get_serial_sequence('vcap.department_roles','id'),GREATEST((SELECT COALESCE(max(id),1) FROM vcap.department_roles),1),TRUE)").execute(&mut **tx).await.map_err(InstitutionError::Database)?;
    }
    Ok(())
}

fn apply_query(table: &str, merge: bool) -> &'static str {
    match (table, merge) {
        ("departments", false | true) => {
            "INSERT INTO vcap.departments SELECT x.department_id::uuid FROM jsonb_to_recordset($1) x(department_id text) ON CONFLICT DO NOTHING"
        }
        ("admins", false) => {
            "INSERT INTO vcap.admins SELECT x.admin_id,x.email,x.name,x.pfp FROM jsonb_to_recordset($1) x(admin_id varchar,email varchar,name varchar,pfp varchar) ON CONFLICT DO NOTHING"
        }
        ("admins", true) => {
            "INSERT INTO vcap.admins SELECT x.admin_id,x.email,x.name,x.pfp FROM jsonb_to_recordset($1) x(admin_id varchar,email varchar,name varchar,pfp varchar) ON CONFLICT(admin_id) DO UPDATE SET email=EXCLUDED.email,name=EXCLUDED.name,pfp=EXCLUDED.pfp"
        }
        ("faculty", false) => {
            "INSERT INTO vcap.faculty SELECT x.faculty_id,x.name,x.email,x.dept_id::uuid,x.honorific,x.designation,x.status FROM jsonb_to_recordset($1) x(faculty_id varchar,name varchar,email varchar,dept_id text,honorific text,designation text,status text) ON CONFLICT DO NOTHING"
        }
        ("faculty", true) => {
            "INSERT INTO vcap.faculty SELECT x.faculty_id,x.name,x.email,x.dept_id::uuid,x.honorific,x.designation,x.status FROM jsonb_to_recordset($1) x(faculty_id varchar,name varchar,email varchar,dept_id text,honorific text,designation text,status text) ON CONFLICT(faculty_id) DO UPDATE SET name=EXCLUDED.name,email=EXCLUDED.email,dept_id=EXCLUDED.dept_id,honorific=EXCLUDED.honorific,designation=EXCLUDED.designation,status=EXCLUDED.status"
        }
        ("programmes", false) => {
            "INSERT INTO vcap.programmes SELECT x.programme_code,x.hod_id FROM jsonb_to_recordset($1) x(programme_code text,hod_id text) ON CONFLICT DO NOTHING"
        }
        ("programmes", true) => {
            "INSERT INTO vcap.programmes SELECT x.programme_code,x.hod_id FROM jsonb_to_recordset($1) x(programme_code text,hod_id text) ON CONFLICT(programme_code) DO UPDATE SET hod_id=EXCLUDED.hod_id"
        }
        ("schools", false | true) => {
            "INSERT INTO vcap.schools SELECT x.school_id FROM jsonb_to_recordset($1) x(school_id text) ON CONFLICT DO NOTHING"
        }
        ("students", false) => {
            "INSERT INTO vcap.students SELECT x.reg_no,x.name,x.email,x.programme_code FROM jsonb_to_recordset($1) x(reg_no varchar,name varchar,email varchar,programme_code text) ON CONFLICT DO NOTHING"
        }
        ("students", true) => {
            "INSERT INTO vcap.students SELECT x.reg_no,x.name,x.email,x.programme_code FROM jsonb_to_recordset($1) x(reg_no varchar,name varchar,email varchar,programme_code text) ON CONFLICT(reg_no) DO UPDATE SET name=EXCLUDED.name,email=EXCLUDED.email,programme_code=EXCLUDED.programme_code"
        }
        ("student_course_registrations", false) => {
            "INSERT INTO vcap.student_course_registrations SELECT x.student_reg_no,x.course_id,x.academic_year,x.semester,x.registration_status FROM jsonb_to_recordset($1) x(student_reg_no text,course_id text,academic_year text,semester text,registration_status text) ON CONFLICT DO NOTHING"
        }
        ("student_course_registrations", true) => {
            "INSERT INTO vcap.student_course_registrations SELECT x.student_reg_no,x.course_id,x.academic_year,x.semester,x.registration_status FROM jsonb_to_recordset($1) x(student_reg_no text,course_id text,academic_year text,semester text,registration_status text) ON CONFLICT(student_reg_no,course_id,academic_year,semester) DO UPDATE SET registration_status=EXCLUDED.registration_status"
        }
        ("faculty_guide_capacity", false) => {
            "INSERT INTO vcap.faculty_guide_capacity SELECT x.capacity_id::uuid,x.faculty_id,x.academic_year,x.ug_max_projects::int,x.pg_max_projects::int,x.integrated_pg_max_projects::int,x.status FROM jsonb_to_recordset($1) x(capacity_id text,faculty_id varchar,academic_year text,ug_max_projects text,pg_max_projects text,integrated_pg_max_projects text,status text) ON CONFLICT DO NOTHING"
        }
        ("faculty_guide_capacity", true) => {
            "INSERT INTO vcap.faculty_guide_capacity SELECT x.capacity_id::uuid,x.faculty_id,x.academic_year,x.ug_max_projects::int,x.pg_max_projects::int,x.integrated_pg_max_projects::int,x.status FROM jsonb_to_recordset($1) x(capacity_id text,faculty_id varchar,academic_year text,ug_max_projects text,pg_max_projects text,integrated_pg_max_projects text,status text) ON CONFLICT(capacity_id) DO UPDATE SET faculty_id=EXCLUDED.faculty_id,academic_year=EXCLUDED.academic_year,ug_max_projects=EXCLUDED.ug_max_projects,pg_max_projects=EXCLUDED.pg_max_projects,integrated_pg_max_projects=EXCLUDED.integrated_pg_max_projects,status=EXCLUDED.status"
        }
        ("department_roles", false) => {
            "INSERT INTO vcap.department_roles SELECT x.id::int,x.dept_id,x.role_type,x.faculty_id FROM jsonb_to_recordset($1) x(id text,dept_id varchar,role_type varchar,faculty_id varchar) ON CONFLICT DO NOTHING"
        }
        ("department_roles", true) => {
            "INSERT INTO vcap.department_roles SELECT x.id::int,x.dept_id,x.role_type,x.faculty_id FROM jsonb_to_recordset($1) x(id text,dept_id varchar,role_type varchar,faculty_id varchar) ON CONFLICT(id) DO UPDATE SET dept_id=EXCLUDED.dept_id,role_type=EXCLUDED.role_type,faculty_id=EXCLUDED.faculty_id"
        }
        ("faculty_roles", false) => {
            "INSERT INTO vcap.faculty_roles SELECT x.role_id::uuid,x.faculty_id,x.role_type,x.school_id,x.department_id::uuid,x.programme_code,x.status FROM jsonb_to_recordset($1) x(role_id text,faculty_id varchar,role_type text,school_id text,department_id text,programme_code text,status text) ON CONFLICT DO NOTHING"
        }
        ("faculty_roles", true) => {
            "INSERT INTO vcap.faculty_roles SELECT x.role_id::uuid,x.faculty_id,x.role_type,x.school_id,x.department_id::uuid,x.programme_code,x.status FROM jsonb_to_recordset($1) x(role_id text,faculty_id varchar,role_type text,school_id text,department_id text,programme_code text,status text) ON CONFLICT(role_id) DO UPDATE SET faculty_id=EXCLUDED.faculty_id,role_type=EXCLUDED.role_type,school_id=EXCLUDED.school_id,department_id=EXCLUDED.department_id,programme_code=EXCLUDED.programme_code,status=EXCLUDED.status"
        }
        ("paper_teams", false) => {
            "INSERT INTO vcap.paper_assignment_groups SELECT x.external_team_key,x.team_name,x.academic_year,x.semester,x.status FROM jsonb_to_recordset($1) x(external_team_key text,team_name text,academic_year text,semester text,status text) ON CONFLICT DO NOTHING"
        }
        ("paper_teams", true) => {
            "INSERT INTO vcap.paper_assignment_groups SELECT x.external_team_key,x.team_name,x.academic_year,x.semester,x.status FROM jsonb_to_recordset($1) x(external_team_key text,team_name text,academic_year text,semester text,status text) ON CONFLICT(external_team_key) DO UPDATE SET team_name=EXCLUDED.team_name,academic_year=EXCLUDED.academic_year,semester=EXCLUDED.semester,status=EXCLUDED.status"
        }
        ("paper_team_writers", false) => {
            "INSERT INTO vcap.paper_assignment_students SELECT x.external_team_key,x.student_reg_no,x.writer_order::int,x.is_leader FROM jsonb_to_recordset($1) x(external_team_key text,student_reg_no varchar,writer_order text,is_leader boolean) ON CONFLICT DO NOTHING"
        }
        ("paper_team_writers", true) => {
            "INSERT INTO vcap.paper_assignment_students SELECT x.external_team_key,x.student_reg_no,x.writer_order::int,x.is_leader FROM jsonb_to_recordset($1) x(external_team_key text,student_reg_no varchar,writer_order text,is_leader boolean) ON CONFLICT(external_team_key,student_reg_no) DO UPDATE SET writer_order=EXCLUDED.writer_order,is_leader=EXCLUDED.is_leader"
        }
        ("paper_team_mentors", false | true) => {
            "INSERT INTO vcap.paper_assignment_mentors SELECT x.external_team_key,x.faculty_id FROM jsonb_to_recordset($1) x(external_team_key text,faculty_id varchar) ON CONFLICT DO NOTHING"
        }
        _ => unreachable!("apply table is fixed"),
    }
}

async fn reconcile_identity_links_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    actor: UserId,
) -> Result<(), InstitutionError> {
    for (people_table, id_column, links_table, required_role) in [
        ("students", "reg_no", "student_user_links", "writer"),
        ("faculty", "faculty_id", "faculty_user_links", "mentor"),
        ("admins", "admin_id", "admin_user_links", "admin"),
    ] {
        let statement = format!(
            r"INSERT INTO vcap.{links_table} ({id_column},user_id,match_method,status,linked_at)
               SELECT candidate.external_id,
                      CASE WHEN candidate.matches=1 AND candidate.people_matches=1 AND candidate.role=$1 THEN candidate.user_id END,
                      'NORMALIZED_EMAIL',
                      CASE WHEN candidate.matches=0 THEN 'UNLINKED' WHEN candidate.matches>1 OR candidate.people_matches>1 THEN 'AMBIGUOUS' WHEN candidate.role<>$1 OR candidate.role IS NULL THEN 'ROLE_INCOMPATIBLE' ELSE 'LINKED' END,
                      CASE WHEN candidate.matches=1 AND candidate.people_matches=1 AND candidate.role=$1 THEN statement_timestamp() END
               FROM (
                   SELECT person.{id_column} AS external_id,account.user_id,account.matches,role.role,
                          count(*) OVER (PARTITION BY lower(btrim(person.email))) AS people_matches
                   FROM vcap.{people_table} person
                   LEFT JOIN LATERAL (
                       SELECT (array_agg(c.user_id ORDER BY c.user_id))[1] AS user_id,count(*) AS matches
                       FROM latex_core.user_credentials c
                       WHERE person.email IS NOT NULL AND lower(btrim(c.email))=lower(btrim(person.email))
                   ) account ON TRUE
                   LEFT JOIN latex_core.global_user_roles role ON role.user_id=account.user_id
               ) candidate
               ON CONFLICT({id_column}) DO UPDATE
               SET user_id=EXCLUDED.user_id,match_method=EXCLUDED.match_method,status=EXCLUDED.status,linked_at=EXCLUDED.linked_at
               WHERE vcap.{links_table}.match_method IS DISTINCT FROM 'MANUAL'"
        );
        sqlx::query(&statement)
            .bind(required_role)
            .execute(&mut **tx)
            .await
            .map_err(InstitutionError::Database)?;
        let counts = format!(
            "SELECT count(*) FILTER (WHERE status='LINKED') AS linked,count(*) FILTER (WHERE status<>'LINKED') AS unlinked FROM vcap.{links_table}"
        );
        let row = sqlx::query(&counts)
            .fetch_one(&mut **tx)
            .await
            .map_err(InstitutionError::Database)?;
        audit_tx(
            tx,
            actor,
            "institution.identity.reconciled",
            links_table,
            Uuid::nil(),
            json!({
                "linked":row.try_get::<i64,_>("linked").map_err(InstitutionError::Database)?,
                "unlinked":row.try_get::<i64,_>("unlinked").map_err(InstitutionError::Database)?,
                "match_method":"NORMALIZED_EMAIL"
            }),
        )
        .await?;
    }
    Ok(())
}

async fn audit_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    actor: UserId,
    event_type: &str,
    resource_type: &str,
    resource_id: Uuid,
    metadata: Value,
) -> Result<(), InstitutionError> {
    sqlx::query("INSERT INTO latex_core.audit_events (id,actor_user_id,event_type,resource_type,resource_id,metadata) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(Uuid::new_v4()).bind(actor.as_uuid()).bind(event_type).bind(resource_type).bind(resource_id).bind(metadata).execute(&mut **tx).await.map_err(InstitutionError::Database)?;
    Ok(())
}

fn decode_job(row: PgRow) -> Result<InstitutionImportJob, InstitutionError> {
    Ok(InstitutionImportJob {
        id: row.try_get("id").map_err(InstitutionError::Database)?,
        import_kind: row
            .try_get("import_kind")
            .map_err(InstitutionError::Database)?,
        mode: row.try_get("mode").map_err(InstitutionError::Database)?,
        original_filename: row
            .try_get("original_filename")
            .map_err(InstitutionError::Database)?,
        content_sha256: row
            .try_get("content_sha256")
            .map_err(InstitutionError::Database)?,
        file_type: row
            .try_get("file_type")
            .map_err(InstitutionError::Database)?,
        submitted_by_user_id: row
            .try_get("submitted_by_user_id")
            .map_err(InstitutionError::Database)?,
        status: row.try_get("status").map_err(InstitutionError::Database)?,
        total_rows: row
            .try_get("total_rows")
            .map_err(InstitutionError::Database)?,
        inserted_rows: row
            .try_get("inserted_rows")
            .map_err(InstitutionError::Database)?,
        updated_rows: row
            .try_get("updated_rows")
            .map_err(InstitutionError::Database)?,
        skipped_rows: row
            .try_get("skipped_rows")
            .map_err(InstitutionError::Database)?,
        error_rows: row
            .try_get("error_rows")
            .map_err(InstitutionError::Database)?,
        created_at: row
            .try_get("created_at")
            .map_err(InstitutionError::Database)?,
        validated_at: row
            .try_get("validated_at")
            .map_err(InstitutionError::Database)?,
        applied_at: row
            .try_get("applied_at")
            .map_err(InstitutionError::Database)?,
    })
}

fn decode_import_row(row: PgRow) -> Result<InstitutionImportRow, InstitutionError> {
    Ok(InstitutionImportRow {
        job_id: row.try_get("job_id").map_err(InstitutionError::Database)?,
        source_table_or_sheet: row
            .try_get("source_table_or_sheet")
            .map_err(InstitutionError::Database)?,
        row_number: row
            .try_get("row_number")
            .map_err(InstitutionError::Database)?,
        natural_key: row
            .try_get("natural_key")
            .map_err(InstitutionError::Database)?,
        payload: row.try_get("payload").map_err(InstitutionError::Database)?,
        action: row.try_get("action").map_err(InstitutionError::Database)?,
        status: row.try_get("status").map_err(InstitutionError::Database)?,
        error_code: row
            .try_get("error_code")
            .map_err(InstitutionError::Database)?,
        error_message: row
            .try_get("error_message")
            .map_err(InstitutionError::Database)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use zip::{ZipWriter, write::SimpleFileOptions};

    #[test]
    fn csv_parser_accepts_valid_students_and_rejects_duplicate_keys() {
        let bytes = b"reg_no,name,email,programme_code\nS1,One,one@example.edu,CSE\nS1,Again,again@example.edu,CSE\n";
        let mut rows =
            parse_csv("students.csv", None, bytes, ImportLimits::default()).expect("CSV parses");
        validate_cross_rows(&mut rows);
        assert!(rows[0].error.is_none());
        assert_eq!(
            rows[1].error.as_ref().map(|value| value.0),
            Some("DUPLICATE_KEY")
        );
    }

    #[test]
    fn csv_parser_rejects_missing_columns_and_malformed_uuid() {
        assert!(
            parse_csv(
                "departments.csv",
                None,
                b"wrong\nvalue\n",
                ImportLimits::default()
            )
            .is_err()
        );
        let rows = parse_csv(
            "departments.csv",
            None,
            b"department_id\nnot-a-uuid\n",
            ImportLimits::default(),
        )
        .expect("shape parses");
        assert_eq!(
            rows[0].error.as_ref().map(|value| value.0),
            Some("MALFORMED_UUID")
        );
    }

    #[test]
    fn tie_break_is_defined_by_writer_order_data_structures() {
        let mut counts = BTreeMap::new();
        counts.insert("ECE", 1);
        counts.insert("CSE", 1);
        let first = HashMap::from([("ECE", 0_usize), ("CSE", 1_usize)]);
        let selected = counts
            .iter()
            .filter(|(_, count)| **count == 1)
            .min_by_key(|(programme, _)| first.get(*programme).copied())
            .map(|(programme, _)| *programme);
        assert_eq!(selected, Some("ECE"));
    }

    #[test]
    fn xlsx_parser_accepts_known_sheet_and_rejects_formula_key() {
        let valid = workbook_bytes(
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>reg_no</t></is></c><c r="B1" t="inlineStr"><is><t>name</t></is></c><c r="C1" t="inlineStr"><is><t>email</t></is></c><c r="D1" t="inlineStr"><is><t>programme_code</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>S1</t></is></c><c r="B2" t="inlineStr"><is><t>Student</t></is></c><c r="C2" t="inlineStr"><is><t>student@example.edu</t></is></c><c r="D2" t="inlineStr"><is><t>CSE</t></is></c></row>"#,
        );
        let rows = parse_xlsx(&valid, ImportLimits::default()).expect("valid XLSX parses");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].error.is_none());

        let formula = workbook_bytes(
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>reg_no</t></is></c><c r="B1" t="inlineStr"><is><t>name</t></is></c><c r="C1" t="inlineStr"><is><t>email</t></is></c><c r="D1" t="inlineStr"><is><t>programme_code</t></is></c></row><row r="2"><c r="A2"><f>CONCAT(&quot;S&quot;,&quot;1&quot;)</f><v>1</v></c><c r="B2" t="inlineStr"><is><t>Student</t></is></c><c r="C2" t="inlineStr"><is><t>student@example.edu</t></is></c><c r="D2" t="inlineStr"><is><t>CSE</t></is></c></row>"#,
        );
        let rows = parse_xlsx(&formula, ImportLimits::default()).expect("XLSX shape parses");
        assert_eq!(
            rows[0].error.as_ref().map(|value| value.0),
            Some("FORMULA_IN_KEY")
        );
    }

    fn workbook_bytes(sheet_rows: &str) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut output);
            for (path, body) in [
                ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#.to_owned()),
                ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_owned()),
                ("xl/workbook.xml", r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="students" sheetId="1" r:id="rId1"/></sheets></workbook>"#.to_owned()),
                ("xl/_rels/workbook.xml.rels", r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.to_owned()),
                ("xl/worksheets/sheet1.xml", format!(r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{sheet_rows}</sheetData></worksheet>"#)),
            ] {
                writer.start_file(path, SimpleFileOptions::default()).expect("ZIP member starts");
                writer.write_all(body.as_bytes()).expect("ZIP member writes");
            }
            writer.finish().expect("XLSX ZIP finishes");
        }
        output.into_inner()
    }
}
