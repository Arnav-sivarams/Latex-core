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
    UpdateOnly,
    DeleteOnly,
}

impl ImportMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ValidateOnly => "VALIDATE_ONLY",
            Self::Merge => "MERGE",
            Self::AddOnly => "ADD_ONLY",
            Self::UpdateOnly => "UPDATE_ONLY",
            Self::DeleteOnly => "DELETE_ONLY",
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
            "UPDATE_ONLY" | "EDIT" => Ok(Self::UpdateOnly),
            "DELETE_ONLY" | "DELETE" => Ok(Self::DeleteOnly),
            _ => Err(InstitutionError::InvalidInput("unknown import mode".into())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InstitutionOperation {
    Add,
    Edit,
    Delete,
}

impl InstitutionOperation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "ADD",
            Self::Edit => "EDIT",
            Self::Delete => "DELETE",
        }
    }

    #[must_use]
    pub const fn mode(self) -> ImportMode {
        match self {
            Self::Add => ImportMode::AddOnly,
            Self::Edit => ImportMode::UpdateOnly,
            Self::Delete => ImportMode::DeleteOnly,
        }
    }
}

impl FromStr for InstitutionOperation {
    type Err = InstitutionError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_uppercase().as_str() {
            "ADD" => Ok(Self::Add),
            "EDIT" => Ok(Self::Edit),
            "DELETE" => Ok(Self::Delete),
            _ => Err(InstitutionError::InvalidInput(
                "operation must be ADD, EDIT, or DELETE".into(),
            )),
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
pub struct InstitutionImportBatch {
    pub id: Uuid,
    pub operation: String,
    pub status: String,
    pub submitted_by_user_id: Uuid,
    pub total_files: i32,
    pub total_rows: i64,
    pub added_rows: i64,
    pub edited_rows: i64,
    pub deleted_rows: i64,
    pub skipped_rows: i64,
    pub error_rows: i64,
    pub created_at: String,
    pub validated_at: Option<String>,
    pub applied_at: Option<String>,
}

#[derive(Clone, Debug)]
pub struct InstitutionBatchUpload {
    pub filename: String,
    pub target_table: Option<String>,
    pub bytes: Vec<u8>,
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
    pub student_count: i64,
    pub template_id: Option<Uuid>,
    pub template_name: Option<String>,
    pub updated_at: Option<String>,
    pub updated_by: Option<String>,
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

#[derive(Clone, Debug, Default)]
pub struct InstitutionPageFilter {
    pub limit: i64,
    pub page: i64,
    pub search: Option<String>,
    pub programme_code: Option<String>,
    pub department_id: Option<Uuid>,
    pub link_status: Option<String>,
    pub external_type: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ImportJobPageFilter {
    pub limit: i64,
    pub page: i64,
    pub search: Option<String>,
    pub status: Option<String>,
    pub mode: Option<String>,
    pub file_type: Option<String>,
}

#[derive(Clone, Debug)]
struct SourceRow {
    upload_index: usize,
    table: String,
    row_number: i64,
    natural_key: Value,
    payload: Value,
    existing_payload: Option<Value>,
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
            ImportFileType::Csv => parse_csv(filename, target_table, bytes, limits, mode, 0)?,
            ImportFileType::Xlsx => {
                if target_table.is_some() {
                    return Err(InstitutionError::InvalidInput(
                        "XLSX target table comes from worksheet names".into(),
                    ));
                }
                parse_xlsx(bytes, limits, mode, 0)?
            }
        };
        validate_cross_rows(&mut rows);
        self.assign_actions(&mut rows, mode).await?;
        self.validate_database_references(&mut rows).await?;
        if mode == ImportMode::DeleteOnly {
            self.validate_delete_dependencies(&mut rows).await?;
        }

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
                 (job_id,source_table_or_sheet,row_number,natural_key,payload,existing_payload,action,status,error_code,error_message) ",
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
                    .push_bind(&row.existing_payload)
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

    pub async fn validate_batch(
        &self,
        submitted_by: UserId,
        operation: InstitutionOperation,
        uploads: &[InstitutionBatchUpload],
        limits: ImportLimits,
    ) -> Result<Value, InstitutionError> {
        if uploads.is_empty() || uploads.len() > 20 {
            return Err(InstitutionError::InvalidInput(
                "select between 1 and 20 CSV/XLSX files".into(),
            ));
        }
        let total_bytes = uploads.iter().try_fold(0_usize, |total, upload| {
            total
                .checked_add(upload.bytes.len())
                .ok_or_else(|| InstitutionError::InvalidInput("batch upload size overflow".into()))
        })?;
        if total_bytes > 64 * 1024 * 1024 {
            return Err(InstitutionError::InvalidInput(
                "the combined batch exceeds the 64 MiB limit".into(),
            ));
        }
        let mut seen_files = HashSet::new();
        let mut rows = Vec::new();
        let mode = operation.mode();
        let mut file_metadata = Vec::with_capacity(uploads.len());
        for (upload_index, upload) in uploads.iter().enumerate() {
            if upload.filename.trim().is_empty() || upload.filename.chars().count() > 255 {
                return Err(InstitutionError::InvalidInput("invalid filename".into()));
            }
            if upload.bytes.is_empty() || upload.bytes.len() > limits.max_upload_bytes {
                return Err(InstitutionError::InvalidInput(format!(
                    "{} is empty or exceeds the per-file limit",
                    upload.filename
                )));
            }
            let digest = hex::encode(Sha256::digest(&upload.bytes));
            if !seen_files.insert((upload.filename.to_ascii_lowercase(), digest.clone())) {
                return Err(InstitutionError::InvalidInput(format!(
                    "{} was selected more than once; remove the duplicate file",
                    upload.filename
                )));
            }
            let file_type = file_type(&upload.filename)?;
            let parsed = match file_type {
                ImportFileType::Csv => parse_csv(
                    &upload.filename,
                    upload.target_table.as_deref(),
                    &upload.bytes,
                    limits,
                    mode,
                    upload_index,
                )?,
                ImportFileType::Xlsx => {
                    if upload.target_table.is_some() {
                        return Err(InstitutionError::InvalidInput(
                            "XLSX datasets are detected from worksheet names".into(),
                        ));
                    }
                    parse_xlsx(&upload.bytes, limits, mode, upload_index)?
                }
            };
            let import_kind = if file_type == ImportFileType::Csv {
                parsed
                    .first()
                    .map_or_else(|| "empty".to_owned(), |row| row.table.clone())
            } else {
                "workbook".to_owned()
            };
            file_metadata.push((file_type, digest, import_kind));
            rows.extend(parsed);
        }
        if rows.len() > 250_000 {
            return Err(InstitutionError::InvalidInput(
                "the combined batch exceeds the 250,000-row limit".into(),
            ));
        }
        validate_cross_rows(&mut rows);
        self.assign_actions(&mut rows, mode).await?;
        self.validate_database_references(&mut rows).await?;
        if operation == InstitutionOperation::Delete {
            self.validate_delete_dependencies(&mut rows).await?;
        }

        let batch_id = Uuid::new_v4();
        let job_ids = (0..uploads.len())
            .map(|_| Uuid::new_v4())
            .collect::<Vec<_>>();
        let total_rows = i64::try_from(rows.len())
            .map_err(|_| InstitutionError::InvalidInput("too many rows".into()))?;
        let error_rows = count_rows(&rows, |row| row.error.is_some())?;
        let added_rows = count_rows(&rows, |row| {
            row.error.is_none() && action_for(row) == "INSERT"
        })?;
        let edited_rows = count_rows(&rows, |row| {
            row.error.is_none() && action_for(row) == "UPDATE"
        })?;
        let deleted_rows = count_rows(&rows, |row| {
            row.error.is_none() && action_for(row) == "DELETE"
        })?;
        let skipped_rows = count_rows(&rows, |row| {
            row.error.is_none() && action_for(row) == "SKIP"
        })?;
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
            "INSERT INTO latex_core.institution_import_batches \
             (id,operation,status,submitted_by_user_id,total_files,total_rows,added_rows,edited_rows,deleted_rows,skipped_rows,error_rows,validated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,statement_timestamp())",
        )
        .bind(batch_id)
        .bind(operation.as_str())
        .bind(status)
        .bind(submitted_by.as_uuid())
        .bind(i32::try_from(uploads.len()).map_err(|_| InstitutionError::InvalidInput("too many files".into()))?)
        .bind(total_rows)
        .bind(added_rows)
        .bind(edited_rows)
        .bind(deleted_rows)
        .bind(skipped_rows)
        .bind(error_rows)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        for (index, upload) in uploads.iter().enumerate() {
            let (file_type, digest, import_kind) = &file_metadata[index];
            let file_rows = rows
                .iter()
                .filter(|row| row.upload_index == index)
                .collect::<Vec<_>>();
            let file_total = i64::try_from(file_rows.len())
                .map_err(|_| InstitutionError::InvalidInput("too many rows".into()))?;
            let file_errors =
                i64::try_from(file_rows.iter().filter(|row| row.error.is_some()).count())
                    .map_err(|_| InstitutionError::InvalidInput("too many errors".into()))?;
            sqlx::query(
                "INSERT INTO latex_core.institution_import_jobs \
                 (id,batch_id,import_kind,mode,original_filename,content_sha256,file_type,submitted_by_user_id,status,total_rows,error_rows,validated_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,statement_timestamp())",
            )
            .bind(job_ids[index])
            .bind(batch_id)
            .bind(import_kind)
            .bind(mode.as_str())
            .bind(&upload.filename)
            .bind(digest)
            .bind(file_type.as_str())
            .bind(submitted_by.as_uuid())
            .bind(if file_errors == 0 { "VALIDATED" } else { "FAILED" })
            .bind(file_total)
            .bind(file_errors)
            .execute(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?;
        }
        for chunk in rows.chunks(500) {
            let mut builder: QueryBuilder<'_, Postgres> = QueryBuilder::new(
                "INSERT INTO latex_core.institution_import_rows \
                 (job_id,source_table_or_sheet,row_number,natural_key,payload,existing_payload,action,status,error_code,error_message) ",
            );
            builder.push_values(chunk, |mut separated, row| {
                let (code, message) = row
                    .error
                    .as_ref()
                    .map_or((None, None), |(code, message)| (Some(*code), Some(message)));
                separated
                    .push_bind(job_ids[row.upload_index])
                    .push_bind(&row.table)
                    .push_bind(row.row_number)
                    .push_bind(&row.natural_key)
                    .push_bind(&row.payload)
                    .push_bind(&row.existing_payload)
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
            "institution.import_batch.validated",
            "institution_import_batch",
            batch_id,
            json!({"operation":operation.as_str(),"total_files":uploads.len(),"total_rows":total_rows,"error_rows":error_rows}),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)?;
        self.batch_detail(batch_id, 100).await
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

        let import_mode = mode.parse::<ImportMode>()?;
        let order = if import_mode == ImportMode::DeleteOnly {
            APPLY_ORDER.iter().rev().copied().collect::<Vec<_>>()
        } else {
            APPLY_ORDER.to_vec()
        };
        for table in order {
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
                apply_table(&mut tx, table, import_mode, Value::Array(payloads)).await?;
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

    pub async fn apply_batch(
        &self,
        batch_id: Uuid,
        actor: UserId,
    ) -> Result<Value, InstitutionError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        let row = sqlx::query(
            "SELECT operation,status,error_rows FROM latex_core.institution_import_batches WHERE id=$1 FOR UPDATE",
        )
        .bind(batch_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?
        .ok_or(InstitutionError::NotFound)?;
        let operation: String = row
            .try_get("operation")
            .map_err(InstitutionError::Database)?;
        let status: String = row.try_get("status").map_err(InstitutionError::Database)?;
        let error_rows: i64 = row
            .try_get("error_rows")
            .map_err(InstitutionError::Database)?;
        if status == "APPLIED" || status == "PARTIAL" {
            tx.rollback().await.map_err(InstitutionError::Database)?;
            return self.batch_detail(batch_id, 100).await;
        }
        if status != "VALIDATED" || error_rows != 0 {
            return Err(InstitutionError::Conflict(
                "only an error-free reviewed batch can be applied".into(),
            ));
        }
        let mode = operation.parse::<InstitutionOperation>()?.mode();
        sqlx::query(
            "UPDATE latex_core.institution_import_batches SET status='APPLYING' WHERE id=$1",
        )
        .bind(batch_id)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        sqlx::query(
            "UPDATE latex_core.institution_import_jobs SET status='APPLYING' WHERE batch_id=$1",
        )
        .bind(batch_id)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        let order = if mode == ImportMode::DeleteOnly {
            APPLY_ORDER.iter().rev().copied().collect::<Vec<_>>()
        } else {
            APPLY_ORDER.to_vec()
        };
        for table in order {
            let payloads: Vec<Value> = sqlx::query_scalar(
                "SELECT row.payload FROM latex_core.institution_import_rows row \
                 JOIN latex_core.institution_import_jobs job ON job.id=row.job_id \
                 WHERE job.batch_id=$1 AND row.source_table_or_sheet=$2 AND row.status='VALID' \
                 AND row.action IN ('INSERT','UPDATE','DELETE') ORDER BY row.row_number",
            )
            .bind(batch_id)
            .bind(table)
            .fetch_all(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?;
            if !payloads.is_empty() {
                apply_table(&mut tx, table, mode, Value::Array(payloads)).await?;
            }
        }
        reconcile_identity_links_tx(&mut tx, actor).await?;
        sqlx::query(
            r"UPDATE latex_core.institution_import_jobs job SET
                 status='APPLIED',
                 inserted_rows=(SELECT count(*) FROM latex_core.institution_import_rows row WHERE row.job_id=job.id AND row.action='INSERT'),
                 updated_rows=(SELECT count(*) FROM latex_core.institution_import_rows row WHERE row.job_id=job.id AND row.action='UPDATE'),
                 skipped_rows=(SELECT count(*) FROM latex_core.institution_import_rows row WHERE row.job_id=job.id AND row.action='SKIP'),
                 applied_at=statement_timestamp()
               WHERE job.batch_id=$1",
        )
        .bind(batch_id)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        sqlx::query(
            "UPDATE latex_core.institution_import_rows row SET status=CASE WHEN action='SKIP' THEN 'SKIPPED' ELSE 'APPLIED' END \
             FROM latex_core.institution_import_jobs job WHERE row.job_id=job.id AND job.batch_id=$1 AND row.status='VALID'",
        )
        .bind(batch_id)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        sqlx::query(
            "UPDATE latex_core.institution_import_batches SET status='APPLIED',applied_at=statement_timestamp() WHERE id=$1",
        )
        .bind(batch_id)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        audit_tx(
            &mut tx,
            actor,
            "institution.import_batch.applied",
            "institution_import_batch",
            batch_id,
            json!({"operation":operation}),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)?;
        self.batch_detail(batch_id, 100).await
    }

    pub async fn batch_primary_job(&self, batch_id: Uuid) -> Result<Uuid, InstitutionError> {
        sqlx::query_scalar(
            r"SELECT job.id FROM latex_core.institution_import_jobs job WHERE job.batch_id=$1
               ORDER BY EXISTS(
                   SELECT 1 FROM latex_core.institution_import_rows row WHERE row.job_id=job.id
                   AND row.source_table_or_sheet IN ('paper_teams','paper_team_writers','paper_team_mentors')
               ) DESC,job.created_at,job.id LIMIT 1",
        )
        .bind(batch_id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?
        .ok_or(InstitutionError::NotFound)
    }

    pub async fn batch_detail(&self, id: Uuid, row_limit: i64) -> Result<Value, InstitutionError> {
        let batch = self.batch(id).await?;
        let files = sqlx::query(
            r"SELECT job.id,job.original_filename,job.content_sha256,job.file_type,job.status,job.total_rows,job.error_rows,
                     COALESCE(array_agg(DISTINCT row.source_table_or_sheet ORDER BY row.source_table_or_sheet) FILTER (WHERE row.source_table_or_sheet IS NOT NULL),'{}') AS datasets
              FROM latex_core.institution_import_jobs job
              LEFT JOIN latex_core.institution_import_rows row ON row.job_id=job.id
              WHERE job.batch_id=$1 GROUP BY job.id ORDER BY job.created_at,job.id",
        )
        .bind(id)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?
        .into_iter()
        .map(|row| {
            Ok(json!({
                "id":row.try_get::<Uuid,_>("id").map_err(InstitutionError::Database)?,
                "filename":row.try_get::<String,_>("original_filename").map_err(InstitutionError::Database)?,
                "checksum":row.try_get::<String,_>("content_sha256").map_err(InstitutionError::Database)?,
                "file_type":row.try_get::<String,_>("file_type").map_err(InstitutionError::Database)?,
                "status":row.try_get::<String,_>("status").map_err(InstitutionError::Database)?,
                "total_rows":row.try_get::<i64,_>("total_rows").map_err(InstitutionError::Database)?,
                "error_rows":row.try_get::<i64,_>("error_rows").map_err(InstitutionError::Database)?,
                "datasets":row.try_get::<Vec<String>,_>("datasets").map_err(InstitutionError::Database)?,
            }))
        })
        .collect::<Result<Vec<_>, InstitutionError>>()?;
        let summaries = sqlx::query(
            r"SELECT row.source_table_or_sheet,count(*) AS total,
                     count(*) FILTER (WHERE row.action='INSERT') AS add_rows,
                     count(*) FILTER (WHERE row.action='UPDATE') AS edit_rows,
                     count(*) FILTER (WHERE row.action='DELETE') AS delete_rows,
                     count(*) FILTER (WHERE row.action='SKIP') AS skip_rows,
                     count(*) FILTER (WHERE row.status IN ('ERROR','UNRESOLVED')) AS error_rows
              FROM latex_core.institution_import_rows row
              JOIN latex_core.institution_import_jobs job ON job.id=row.job_id
              WHERE job.batch_id=$1 GROUP BY row.source_table_or_sheet ORDER BY row.source_table_or_sheet",
        )
        .bind(id)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?
        .into_iter()
        .map(|row| {
            let source: String = row.try_get("source_table_or_sheet").map_err(InstitutionError::Database)?;
            Ok(json!({
                "source":source,"dataset":friendly_dataset(&source),
                "total":row.try_get::<i64,_>("total").map_err(InstitutionError::Database)?,
                "add":row.try_get::<i64,_>("add_rows").map_err(InstitutionError::Database)?,
                "edit":row.try_get::<i64,_>("edit_rows").map_err(InstitutionError::Database)?,
                "delete":row.try_get::<i64,_>("delete_rows").map_err(InstitutionError::Database)?,
                "skip":row.try_get::<i64,_>("skip_rows").map_err(InstitutionError::Database)?,
                "error":row.try_get::<i64,_>("error_rows").map_err(InstitutionError::Database)?,
            }))
        })
        .collect::<Result<Vec<_>, InstitutionError>>()?;
        let issues = sqlx::query(
            r"SELECT job.original_filename,row.source_table_or_sheet,row.row_number,row.natural_key,row.payload,row.existing_payload,row.error_code,row.error_message
              FROM latex_core.institution_import_rows row JOIN latex_core.institution_import_jobs job ON job.id=row.job_id
              WHERE job.batch_id=$1 AND row.status IN ('ERROR','UNRESOLVED')
              ORDER BY job.created_at,row.source_table_or_sheet,row.row_number LIMIT $2",
        )
        .bind(id)
        .bind(row_limit.clamp(1, 200))
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?
        .into_iter()
        .map(|row| {
            let source: String = row.try_get("source_table_or_sheet").map_err(InstitutionError::Database)?;
            Ok(json!({
                "file":row.try_get::<String,_>("original_filename").map_err(InstitutionError::Database)?,
                "row":row.try_get::<i64,_>("row_number").map_err(InstitutionError::Database)?,
                "dataset":friendly_dataset(&source),"source":source,
                "key":row.try_get::<Value,_>("natural_key").map_err(InstitutionError::Database)?,
                "payload":row.try_get::<Value,_>("payload").map_err(InstitutionError::Database)?,
                "existing":row.try_get::<Option<Value>,_>("existing_payload").map_err(InstitutionError::Database)?,
                "code":row.try_get::<Option<String>,_>("error_code").map_err(InstitutionError::Database)?,
                "problem":row.try_get::<Option<String>,_>("error_message").map_err(InstitutionError::Database)?,
                "suggested_action":friendly_suggestion(row.try_get::<Option<String>,_>("error_code").map_err(InstitutionError::Database)?.as_deref()),
            }))
        })
        .collect::<Result<Vec<_>, InstitutionError>>()?;
        let changes = sqlx::query(
            r"SELECT row.source_table_or_sheet,row.natural_key,row.payload,row.existing_payload
              FROM latex_core.institution_import_rows row JOIN latex_core.institution_import_jobs job ON job.id=row.job_id
              WHERE job.batch_id=$1 AND row.action='UPDATE' AND row.status NOT IN ('ERROR','UNRESOLVED')
              ORDER BY job.created_at,row.source_table_or_sheet,row.row_number LIMIT $2",
        )
        .bind(id)
        .bind(row_limit.clamp(1, 200))
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?
        .into_iter()
        .map(|row| {
            let source: String = row
                .try_get("source_table_or_sheet")
                .map_err(InstitutionError::Database)?;
            let natural_key: Value = row
                .try_get("natural_key")
                .map_err(InstitutionError::Database)?;
            let payload: Value = row.try_get("payload").map_err(InstitutionError::Database)?;
            let existing: Value = row
                .try_get::<Option<Value>, _>("existing_payload")
                .map_err(InstitutionError::Database)?
                .unwrap_or_else(|| json!({}));
            let mut fields = Map::new();
            if let Some(values) = payload.as_object() {
                for (field, new_value) in values {
                    if field == "__action" || spec(&source).keys.contains(&field.as_str()) {
                        continue;
                    }
                    let old_value = existing.get(field).cloned().unwrap_or(Value::Null);
                    if old_value != *new_value {
                        fields.insert(
                            field.clone(),
                            json!({"old":old_value,"new":new_value}),
                        );
                    }
                }
            }
            let display_key = natural_key.as_object().map_or_else(String::new, |values| {
                values
                    .values()
                    .map(|value| {
                        value
                            .as_str()
                            .map_or_else(|| value.to_string(), str::to_owned)
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            });
            let template_effect = (source == "students"
                && fields.contains_key("programme_code"))
            .then_some("Programme changed. Existing Team template remains pinned.");
            Ok(json!({
                "dataset":friendly_dataset(&source),"source":source,"display_key":display_key,
                "fields":fields,"template_effect":template_effect,
            }))
        })
        .collect::<Result<Vec<_>, InstitutionError>>()?;
        Ok(json!({
            "batch":batch,"files":files,"summaries":summaries,"issues":issues,"changes":changes,
            "issues_limited":true,"changes_limited":true
        }))
    }

    pub async fn batch(&self, id: Uuid) -> Result<InstitutionImportBatch, InstitutionError> {
        let row = sqlx::query(
            "SELECT id,operation,status,submitted_by_user_id,total_files,total_rows,added_rows,edited_rows,deleted_rows,skipped_rows,error_rows,created_at::text,validated_at::text,applied_at::text FROM latex_core.institution_import_batches WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?
        .ok_or(InstitutionError::NotFound)?;
        decode_batch(row)
    }

    pub async fn paginated_batches(
        &self,
        filter: &ImportJobPageFilter,
    ) -> Result<PaperTeamPage, InstitutionError> {
        let limit = page_limit(filter.limit);
        let page = filter.page.max(1);
        let offset = (page - 1).saturating_mul(limit);
        let rows = sqlx::query(
            r"SELECT batch.id,batch.operation,batch.status,batch.total_files,batch.total_rows,batch.added_rows,batch.edited_rows,batch.deleted_rows,batch.skipped_rows,batch.error_rows,batch.created_at::text AS created_at,
                     string_agg(job.original_filename, ', ' ORDER BY job.created_at,job.id) AS filenames,count(*) OVER() AS total
              FROM latex_core.institution_import_batches batch
              JOIN latex_core.institution_import_jobs job ON job.batch_id=batch.id
              WHERE ($1::text IS NULL OR job.original_filename ILIKE '%' || $1 || '%' OR batch.id::text ILIKE '%' || $1 || '%')
                AND ($2::text IS NULL OR batch.status=$2)
                AND ($3::text IS NULL OR batch.operation=$3)
              GROUP BY batch.id ORDER BY batch.created_at DESC,batch.id DESC LIMIT $4 OFFSET $5",
        )
        .bind(clean_filter(filter.search.as_deref()))
        .bind(clean_filter(filter.status.as_deref()))
        .bind(clean_filter(filter.mode.as_deref()))
        .bind(limit)
        .bind(offset)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        page_from_rows(rows, page, limit, offset, |row| {
            Ok(json!({
                "id":row.try_get::<Uuid,_>("id").map_err(InstitutionError::Database)?,
                "operation":row.try_get::<String,_>("operation").map_err(InstitutionError::Database)?,
                "status":row.try_get::<String,_>("status").map_err(InstitutionError::Database)?,
                "total_files":row.try_get::<i32,_>("total_files").map_err(InstitutionError::Database)?,
                "total_rows":row.try_get::<i64,_>("total_rows").map_err(InstitutionError::Database)?,
                "added_rows":row.try_get::<i64,_>("added_rows").map_err(InstitutionError::Database)?,
                "edited_rows":row.try_get::<i64,_>("edited_rows").map_err(InstitutionError::Database)?,
                "deleted_rows":row.try_get::<i64,_>("deleted_rows").map_err(InstitutionError::Database)?,
                "skipped_rows":row.try_get::<i64,_>("skipped_rows").map_err(InstitutionError::Database)?,
                "error_rows":row.try_get::<i64,_>("error_rows").map_err(InstitutionError::Database)?,
                "created_at":row.try_get::<String,_>("created_at").map_err(InstitutionError::Database)?,
                "filenames":row.try_get::<String,_>("filenames").map_err(InstitutionError::Database)?,
            }))
        })
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

    pub async fn paginated_jobs(
        &self,
        filter: &ImportJobPageFilter,
    ) -> Result<PaperTeamPage, InstitutionError> {
        let limit = page_limit(filter.limit);
        let page = filter.page.max(1);
        let offset = (page - 1).saturating_mul(limit);
        let rows = sqlx::query(
            r"SELECT j.id,j.batch_id,j.mode,j.original_filename,j.file_type,j.status,j.total_rows,
                     j.inserted_rows,j.updated_rows,j.skipped_rows,j.error_rows,
                     j.created_at::text AS created_at,c.email AS submitted_by,
                     count(*) OVER() AS total
              FROM latex_core.institution_import_jobs j
              JOIN latex_core.user_credentials c ON c.user_id=j.submitted_by_user_id
              WHERE ($1::text IS NULL OR j.original_filename ILIKE '%' || $1 || '%' OR j.id::text ILIKE '%' || $1 || '%')
                AND ($2::text IS NULL OR j.status=$2)
                AND ($3::text IS NULL OR j.mode=$3)
                AND ($4::text IS NULL OR j.file_type=$4)
              ORDER BY j.created_at DESC,j.id DESC LIMIT $5 OFFSET $6",
        )
        .bind(clean_filter(filter.search.as_deref()))
        .bind(clean_filter(filter.status.as_deref()))
        .bind(clean_filter(filter.mode.as_deref()))
        .bind(clean_filter(filter.file_type.as_deref()))
        .bind(limit)
        .bind(offset)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        page_from_rows(rows, page, limit, offset, |row| {
            Ok(json!({
                "id": row.try_get::<Uuid,_>("id").map_err(InstitutionError::Database)?,
                "batch_id": row.try_get::<Option<Uuid>,_>("batch_id").map_err(InstitutionError::Database)?,
                "mode": row.try_get::<String,_>("mode").map_err(InstitutionError::Database)?,
                "original_filename": row.try_get::<String,_>("original_filename").map_err(InstitutionError::Database)?,
                "file_type": row.try_get::<String,_>("file_type").map_err(InstitutionError::Database)?,
                "status": row.try_get::<String,_>("status").map_err(InstitutionError::Database)?,
                "total_rows": row.try_get::<i64,_>("total_rows").map_err(InstitutionError::Database)?,
                "inserted_rows": row.try_get::<i64,_>("inserted_rows").map_err(InstitutionError::Database)?,
                "updated_rows": row.try_get::<i64,_>("updated_rows").map_err(InstitutionError::Database)?,
                "skipped_rows": row.try_get::<i64,_>("skipped_rows").map_err(InstitutionError::Database)?,
                "error_rows": row.try_get::<i64,_>("error_rows").map_err(InstitutionError::Database)?,
                "created_at": row.try_get::<String,_>("created_at").map_err(InstitutionError::Database)?,
                "submitted_by": row.try_get::<String,_>("submitted_by").map_err(InstitutionError::Database)?,
            }))
        })
    }

    pub async fn job_detail(&self, id: Uuid, row_limit: i64) -> Result<Value, InstitutionError> {
        let job = self.job(id).await?;
        let summaries = sqlx::query(
            r"SELECT source_table_or_sheet,
                     count(*) AS total,
                     count(*) FILTER (WHERE action='INSERT') AS insert_rows,
                     count(*) FILTER (WHERE action='UPDATE') AS update_rows,
                     count(*) FILTER (WHERE action='SKIP') AS skip_rows,
                     count(*) FILTER (WHERE status IN ('ERROR','UNRESOLVED')) AS error_rows
              FROM latex_core.institution_import_rows WHERE job_id=$1
              GROUP BY source_table_or_sheet ORDER BY source_table_or_sheet",
        )
        .bind(id)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        let summaries = summaries
            .into_iter()
            .map(|row| {
                Ok(json!({
                    "source":row.try_get::<String,_>("source_table_or_sheet").map_err(InstitutionError::Database)?,
                    "total":row.try_get::<i64,_>("total").map_err(InstitutionError::Database)?,
                    "insert":row.try_get::<i64,_>("insert_rows").map_err(InstitutionError::Database)?,
                    "update":row.try_get::<i64,_>("update_rows").map_err(InstitutionError::Database)?,
                    "skip":row.try_get::<i64,_>("skip_rows").map_err(InstitutionError::Database)?,
                    "error":row.try_get::<i64,_>("error_rows").map_err(InstitutionError::Database)?,
                }))
            })
            .collect::<Result<Vec<_>, InstitutionError>>()?;
        let preview_rows = sqlx::query(
            "SELECT job_id,source_table_or_sheet,row_number,natural_key,payload,action,status,error_code,error_message FROM latex_core.institution_import_rows WHERE job_id=$1 ORDER BY (status IN ('ERROR','UNRESOLVED')) DESC,source_table_or_sheet,row_number LIMIT $2",
        )
        .bind(id)
        .bind(row_limit.clamp(1, 200))
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?
        .into_iter()
        .map(decode_import_row)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({"job":job,"summaries":summaries,"rows":preview_rows,"rows_limited":true}))
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
            "SELECT p.programme_code,count(s.reg_no) AS student_count,d.template_id,t.name AS template_name,d.updated_at::text,u.email AS updated_by \
             FROM vcap.programmes p LEFT JOIN vcap.students s USING(programme_code) \
             LEFT JOIN latex_core.programme_template_defaults d USING(programme_code) \
             LEFT JOIN latex_core.templates t ON t.id=d.template_id \
             LEFT JOIN latex_core.user_credentials u ON u.user_id=d.updated_by_user_id \
             GROUP BY p.programme_code,d.template_id,t.name,d.updated_at,u.email ORDER BY p.programme_code",
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
                    student_count: row
                        .try_get("student_count")
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
                    updated_by: row
                        .try_get("updated_by")
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
        require_materializable_template(&mut tx, template_id).await?;
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
        require_materializable_template(&mut tx, template_id).await?;
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

    pub async fn global_fallback_detail(&self) -> Result<Value, InstitutionError> {
        let row = sqlx::query(
            r"SELECT c.global_fallback_template_id,t.name AS template_name,
                     c.updated_at::text AS updated_at,u.email AS updated_by
              FROM latex_core.institution_template_config c
              LEFT JOIN latex_core.templates t ON t.id=c.global_fallback_template_id
              LEFT JOIN latex_core.user_credentials u ON u.user_id=c.updated_by_user_id
              WHERE c.singleton",
        )
        .fetch_one(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        Ok(json!({
            "template_id":row.try_get::<Option<Uuid>,_>("global_fallback_template_id").map_err(InstitutionError::Database)?,
            "template_name":row.try_get::<Option<String>,_>("template_name").map_err(InstitutionError::Database)?,
            "updated_at":row.try_get::<Option<String>,_>("updated_at").map_err(InstitutionError::Database)?,
            "updated_by":row.try_get::<Option<String>,_>("updated_by").map_err(InstitutionError::Database)?,
        }))
    }

    pub async fn paginated_students(
        &self,
        filter: &InstitutionPageFilter,
    ) -> Result<PaperTeamPage, InstitutionError> {
        let limit = page_limit(filter.limit);
        let page = filter.page.max(1);
        let offset = (page - 1).saturating_mul(limit);
        let rows = sqlx::query(
            r"SELECT s.reg_no,s.name,s.email,s.programme_code,l.user_id,l.status AS link_status,
                     u.email AS v2_email,count(*) OVER() AS total
              FROM vcap.students s LEFT JOIN vcap.student_user_links l USING(reg_no)
              LEFT JOIN latex_core.user_credentials u ON u.user_id=l.user_id
              WHERE ($1::text IS NULL OR s.reg_no ILIKE '%' || $1 || '%' OR s.name ILIKE '%' || $1 || '%' OR s.email ILIKE '%' || $1 || '%')
                AND ($2::text IS NULL OR s.programme_code=$2)
                AND ($3::text IS NULL OR COALESCE(l.status,'UNLINKED')=$3)
              ORDER BY s.reg_no LIMIT $4 OFFSET $5",
        )
        .bind(clean_filter(filter.search.as_deref()))
        .bind(clean_filter(filter.programme_code.as_deref()))
        .bind(clean_filter(filter.link_status.as_deref()))
        .bind(limit)
        .bind(offset)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        page_from_rows(rows, page, limit, offset, |row| {
            Ok(json!({
                "external_type":"STUDENT",
                "external_id":row.try_get::<String,_>("reg_no").map_err(InstitutionError::Database)?,
                "registration_number":row.try_get::<String,_>("reg_no").map_err(InstitutionError::Database)?,
                "name":row.try_get::<Option<String>,_>("name").map_err(InstitutionError::Database)?,
                "email":row.try_get::<Option<String>,_>("email").map_err(InstitutionError::Database)?,
                "programme_code":row.try_get::<Option<String>,_>("programme_code").map_err(InstitutionError::Database)?,
                "user_id":row.try_get::<Option<Uuid>,_>("user_id").map_err(InstitutionError::Database)?,
                "v2_account":row.try_get::<Option<String>,_>("v2_email").map_err(InstitutionError::Database)?,
                "link_status":row.try_get::<Option<String>,_>("link_status").map_err(InstitutionError::Database)?.unwrap_or_else(|| "UNLINKED".into()),
            }))
        })
    }

    pub async fn paginated_faculty(
        &self,
        filter: &InstitutionPageFilter,
    ) -> Result<PaperTeamPage, InstitutionError> {
        let limit = page_limit(filter.limit);
        let page = filter.page.max(1);
        let offset = (page - 1).saturating_mul(limit);
        let rows = sqlx::query(
            r"SELECT f.faculty_id,f.name,f.email,f.dept_id,f.designation,f.status,
                     l.user_id,l.status AS link_status,u.email AS v2_email,
                     COALESCE((SELECT jsonb_agg(jsonb_build_object('academic_year',c.academic_year,'ug',c.ug_max_projects,'pg',c.pg_max_projects,'integrated_pg',c.integrated_pg_max_projects,'status',c.status) ORDER BY c.academic_year DESC) FROM vcap.faculty_guide_capacity c WHERE c.faculty_id=f.faculty_id),'[]'::jsonb) AS guide_capacity,
                     count(*) OVER() AS total
              FROM vcap.faculty f LEFT JOIN vcap.faculty_user_links l USING(faculty_id)
              LEFT JOIN latex_core.user_credentials u ON u.user_id=l.user_id
              WHERE ($1::text IS NULL OR f.faculty_id ILIKE '%' || $1 || '%' OR f.name ILIKE '%' || $1 || '%' OR f.email ILIKE '%' || $1 || '%')
                AND ($2::uuid IS NULL OR f.dept_id=$2)
                AND ($3::text IS NULL OR COALESCE(l.status,'UNLINKED')=$3)
              ORDER BY f.faculty_id LIMIT $4 OFFSET $5",
        )
        .bind(clean_filter(filter.search.as_deref()))
        .bind(filter.department_id)
        .bind(clean_filter(filter.link_status.as_deref()))
        .bind(limit)
        .bind(offset)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        page_from_rows(rows, page, limit, offset, |row| {
            Ok(json!({
                "external_type":"FACULTY",
                "external_id":row.try_get::<String,_>("faculty_id").map_err(InstitutionError::Database)?,
                "faculty_id":row.try_get::<String,_>("faculty_id").map_err(InstitutionError::Database)?,
                "name":row.try_get::<Option<String>,_>("name").map_err(InstitutionError::Database)?,
                "email":row.try_get::<Option<String>,_>("email").map_err(InstitutionError::Database)?,
                "department_id":row.try_get::<Option<Uuid>,_>("dept_id").map_err(InstitutionError::Database)?,
                "designation":row.try_get::<Option<String>,_>("designation").map_err(InstitutionError::Database)?,
                "status":row.try_get::<Option<String>,_>("status").map_err(InstitutionError::Database)?,
                "user_id":row.try_get::<Option<Uuid>,_>("user_id").map_err(InstitutionError::Database)?,
                "v2_account":row.try_get::<Option<String>,_>("v2_email").map_err(InstitutionError::Database)?,
                "link_status":row.try_get::<Option<String>,_>("link_status").map_err(InstitutionError::Database)?.unwrap_or_else(|| "UNLINKED".into()),
                "guide_capacity":row.try_get::<Value,_>("guide_capacity").map_err(InstitutionError::Database)?,
            }))
        })
    }

    pub async fn paginated_programmes(
        &self,
        filter: &InstitutionPageFilter,
    ) -> Result<PaperTeamPage, InstitutionError> {
        let limit = page_limit(filter.limit);
        let page = filter.page.max(1);
        let offset = (page - 1).saturating_mul(limit);
        let rows = sqlx::query(
            r"SELECT p.programme_code,p.hod_id,f.name AS hod_name,
                     count(s.reg_no) AS student_count,d.template_id,t.name AS template_name,
                     count(*) OVER() AS total
              FROM vcap.programmes p LEFT JOIN vcap.faculty f ON f.faculty_id=p.hod_id
              LEFT JOIN vcap.students s USING(programme_code)
              LEFT JOIN latex_core.programme_template_defaults d USING(programme_code)
              LEFT JOIN latex_core.templates t ON t.id=d.template_id
              WHERE ($1::text IS NULL OR p.programme_code ILIKE '%' || $1 || '%' OR f.name ILIKE '%' || $1 || '%')
              GROUP BY p.programme_code,p.hod_id,f.name,d.template_id,t.name
              ORDER BY p.programme_code LIMIT $2 OFFSET $3",
        )
        .bind(clean_filter(filter.search.as_deref()))
        .bind(limit)
        .bind(offset)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        page_from_rows(rows, page, limit, offset, |row| {
            Ok(json!({
                "programme_code":row.try_get::<String,_>("programme_code").map_err(InstitutionError::Database)?,
                "hod_id":row.try_get::<Option<String>,_>("hod_id").map_err(InstitutionError::Database)?,
                "hod_name":row.try_get::<Option<String>,_>("hod_name").map_err(InstitutionError::Database)?,
                "student_count":row.try_get::<i64,_>("student_count").map_err(InstitutionError::Database)?,
                "template_id":row.try_get::<Option<Uuid>,_>("template_id").map_err(InstitutionError::Database)?,
                "template_name":row.try_get::<Option<String>,_>("template_name").map_err(InstitutionError::Database)?,
            }))
        })
    }

    pub async fn paginated_dataset(
        &self,
        dataset: &str,
        filter: &InstitutionPageFilter,
    ) -> Result<PaperTeamPage, InstitutionError> {
        if dataset == "students" {
            let mut page = self.paginated_students(filter).await?;
            for item in &mut page.items {
                if let Some(values) = item.as_object_mut() {
                    if let Some(registration_number) = values.remove("registration_number") {
                        values.insert("reg_no".into(), registration_number);
                    }
                }
            }
            return Ok(page);
        }
        if dataset == "faculty" {
            let mut page = self.paginated_faculty(filter).await?;
            for item in &mut page.items {
                if let Some(values) = item.as_object_mut() {
                    if let Some(department_id) = values.remove("department_id") {
                        values.insert("dept_id".into(), department_id);
                    }
                }
            }
            return Ok(page);
        }
        if dataset == "programmes" {
            return self.paginated_programmes(filter).await;
        }
        let limit = page_limit(filter.limit);
        let page = filter.page.max(1);
        let offset = (page - 1).saturating_mul(limit);
        let query = match dataset {
            "departments" => {
                "SELECT to_jsonb(record) AS payload,count(*) OVER() AS total FROM vcap.departments record WHERE ($1::text IS NULL OR record.department_id::text ILIKE '%' || $1 || '%') ORDER BY record.department_id LIMIT $2 OFFSET $3"
            }
            "schools" => {
                "SELECT to_jsonb(record) AS payload,count(*) OVER() AS total FROM vcap.schools record WHERE ($1::text IS NULL OR record.school_id ILIKE '%' || $1 || '%') ORDER BY record.school_id LIMIT $2 OFFSET $3"
            }
            "student_course_registrations" => {
                "SELECT to_jsonb(record) AS payload,count(*) OVER() AS total FROM vcap.student_course_registrations record WHERE ($1::text IS NULL OR record.student_reg_no ILIKE '%' || $1 || '%' OR record.course_id ILIKE '%' || $1 || '%' OR record.academic_year ILIKE '%' || $1 || '%' OR record.semester ILIKE '%' || $1 || '%') ORDER BY record.student_reg_no,record.academic_year,record.semester,record.course_id LIMIT $2 OFFSET $3"
            }
            "faculty_guide_capacity" => {
                "SELECT to_jsonb(record) AS payload,count(*) OVER() AS total FROM vcap.faculty_guide_capacity record WHERE ($1::text IS NULL OR record.capacity_id::text ILIKE '%' || $1 || '%' OR record.faculty_id ILIKE '%' || $1 || '%' OR record.academic_year ILIKE '%' || $1 || '%') ORDER BY record.academic_year DESC,record.faculty_id,record.capacity_id LIMIT $2 OFFSET $3"
            }
            "department_roles" => {
                "SELECT to_jsonb(record) AS payload,count(*) OVER() AS total FROM vcap.department_roles record WHERE ($1::text IS NULL OR record.id::text ILIKE '%' || $1 || '%' OR record.dept_id ILIKE '%' || $1 || '%' OR record.faculty_id ILIKE '%' || $1 || '%' OR record.role_type ILIKE '%' || $1 || '%') ORDER BY record.dept_id,record.role_type,record.id LIMIT $2 OFFSET $3"
            }
            "faculty_roles" => {
                "SELECT to_jsonb(record) AS payload,count(*) OVER() AS total FROM vcap.faculty_roles record WHERE ($1::text IS NULL OR record.role_id::text ILIKE '%' || $1 || '%' OR record.faculty_id ILIKE '%' || $1 || '%' OR record.role_type ILIKE '%' || $1 || '%' OR record.programme_code ILIKE '%' || $1 || '%') ORDER BY record.faculty_id,record.role_type,record.role_id LIMIT $2 OFFSET $3"
            }
            "paper_teams" => {
                r"SELECT jsonb_build_object(
                    'external_team_key',record.external_team_key,'team_name',record.team_name,
                    'academic_year',record.academic_year,'semester',record.semester,'status',record.status,
                    'writers',(SELECT COALESCE(jsonb_agg(jsonb_build_object('student_reg_no',writer.student_reg_no,'writer_order',writer.writer_order,'is_leader',writer.is_leader) ORDER BY writer.writer_order),'[]') FROM vcap.paper_assignment_students writer WHERE writer.external_team_key=record.external_team_key),
                    'mentors',(SELECT COALESCE(jsonb_agg(jsonb_build_object('faculty_id',mentor.faculty_id) ORDER BY mentor.faculty_id),'[]') FROM vcap.paper_assignment_mentors mentor WHERE mentor.external_team_key=record.external_team_key),
                    'materialized_paper_team_id',link.paper_team_id) AS payload,count(*) OVER() AS total
                FROM vcap.paper_assignment_groups record LEFT JOIN latex_core.external_paper_team_links link USING(external_team_key)
                WHERE ($1::text IS NULL OR record.external_team_key ILIKE '%' || $1 || '%' OR record.team_name ILIKE '%' || $1 || '%'
                    OR EXISTS(SELECT 1 FROM vcap.paper_assignment_students writer WHERE writer.external_team_key=record.external_team_key AND writer.student_reg_no ILIKE '%' || $1 || '%')
                    OR EXISTS(SELECT 1 FROM vcap.paper_assignment_mentors mentor WHERE mentor.external_team_key=record.external_team_key AND mentor.faculty_id ILIKE '%' || $1 || '%'))
                ORDER BY record.external_team_key LIMIT $2 OFFSET $3"
            }
            _ => {
                return Err(InstitutionError::InvalidInput(
                    "unknown institution dataset".into(),
                ));
            }
        };
        let rows = sqlx::query(query)
            .bind(clean_filter(filter.search.as_deref()))
            .bind(limit)
            .bind(offset)
            .fetch_all(self.database.pool())
            .await
            .map_err(InstitutionError::Database)?;
        page_from_rows(rows, page, limit, offset, |row| {
            row.try_get::<Value, _>("payload")
                .map_err(InstitutionError::Database)
        })
    }

    pub async fn validate_manual_operation(
        &self,
        actor: UserId,
        dataset: &str,
        operation: InstitutionOperation,
        payload: &Value,
    ) -> Result<Value, InstitutionError> {
        if !INSTITUTION_TABLES.contains(&dataset) {
            return Err(InstitutionError::InvalidInput(
                "unknown institution dataset".into(),
            ));
        }
        let values = payload.as_object().ok_or_else(|| {
            InstitutionError::InvalidInput("manual record must be a JSON object".into())
        })?;
        if values.is_empty() {
            return Err(InstitutionError::InvalidInput(
                "manual record has no fields".into(),
            ));
        }
        for field in values.keys() {
            if !spec(dataset).columns.contains(&field.as_str()) {
                return Err(InstitutionError::InvalidInput(format!(
                    "{dataset}: unknown field {field}"
                )));
            }
        }
        let headers = values.keys().cloned().collect::<Vec<_>>();
        let record = headers
            .iter()
            .map(|field| match values.get(field) {
                Some(Value::Null) | None => String::new(),
                Some(Value::String(value)) => value.clone(),
                Some(value) => value.to_string(),
            })
            .collect::<Vec<_>>();
        let mut writer = csv::Writer::from_writer(Vec::new());
        writer.write_record(&headers).map_err(|error| {
            InstitutionError::InvalidInput(format!("could not encode manual record: {error}"))
        })?;
        writer.write_record(&record).map_err(|error| {
            InstitutionError::InvalidInput(format!("could not encode manual record: {error}"))
        })?;
        let bytes = writer.into_inner().map_err(|error| {
            InstitutionError::InvalidInput(format!("could not encode manual record: {error}"))
        })?;
        self.validate_batch(
            actor,
            operation,
            &[InstitutionBatchUpload {
                filename: format!("Manual {dataset}.csv"),
                target_table: Some(dataset.to_owned()),
                bytes,
            }],
            ImportLimits::default(),
        )
        .await
    }

    pub async fn paginated_identity_links(
        &self,
        filter: &InstitutionPageFilter,
    ) -> Result<PaperTeamPage, InstitutionError> {
        let limit = page_limit(filter.limit);
        let page = filter.page.max(1);
        let offset = (page - 1).saturating_mul(limit);
        let rows = sqlx::query(
            r"WITH links AS (
                SELECT 'STUDENT'::text AS external_type,s.reg_no::text AS external_id,s.name,s.email,l.user_id,COALESCE(l.status,'UNLINKED') AS status,l.match_method,u.email AS v2_email
                  FROM vcap.students s LEFT JOIN vcap.student_user_links l USING(reg_no) LEFT JOIN latex_core.user_credentials u ON u.user_id=l.user_id
                UNION ALL
                SELECT 'FACULTY',f.faculty_id::text,f.name,f.email,l.user_id,COALESCE(l.status,'UNLINKED'),l.match_method,u.email
                  FROM vcap.faculty f LEFT JOIN vcap.faculty_user_links l USING(faculty_id) LEFT JOIN latex_core.user_credentials u ON u.user_id=l.user_id
                UNION ALL
                SELECT 'ADMIN',a.admin_id::text,a.name,a.email,l.user_id,COALESCE(l.status,'UNLINKED'),l.match_method,u.email
                  FROM vcap.admins a LEFT JOIN vcap.admin_user_links l USING(admin_id) LEFT JOIN latex_core.user_credentials u ON u.user_id=l.user_id
              ) SELECT *,count(*) OVER() AS total FROM links
              WHERE ($1::text IS NULL OR external_id ILIKE '%' || $1 || '%' OR name ILIKE '%' || $1 || '%' OR email ILIKE '%' || $1 || '%' OR v2_email ILIKE '%' || $1 || '%')
                AND ($2::text IS NULL OR status=$2) AND ($3::text IS NULL OR external_type=$3)
              ORDER BY external_type,external_id LIMIT $4 OFFSET $5",
        )
        .bind(clean_filter(filter.search.as_deref()))
        .bind(clean_filter(filter.link_status.as_deref()))
        .bind(clean_filter(filter.external_type.as_deref()))
        .bind(limit)
        .bind(offset)
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        page_from_rows(rows, page, limit, offset, |row| {
            Ok(json!({
                "external_type":row.try_get::<String,_>("external_type").map_err(InstitutionError::Database)?,
                "external_id":row.try_get::<String,_>("external_id").map_err(InstitutionError::Database)?,
                "name":row.try_get::<Option<String>,_>("name").map_err(InstitutionError::Database)?,
                "email":row.try_get::<Option<String>,_>("email").map_err(InstitutionError::Database)?,
                "user_id":row.try_get::<Option<Uuid>,_>("user_id").map_err(InstitutionError::Database)?,
                "v2_account":row.try_get::<Option<String>,_>("v2_email").map_err(InstitutionError::Database)?,
                "status":row.try_get::<String,_>("status").map_err(InstitutionError::Database)?,
                "match_method":row.try_get::<Option<String>,_>("match_method").map_err(InstitutionError::Database)?,
            }))
        })
    }

    pub async fn search_v2_users(
        &self,
        search: &str,
        role: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Value>, InstitutionError> {
        let rows = sqlx::query(
            r"SELECT c.user_id,c.email,r.role FROM latex_core.user_credentials c
              JOIN latex_core.global_user_roles r ON r.user_id=c.user_id
              WHERE c.enabled AND ($1='' OR c.email ILIKE '%' || $1 || '%' OR c.user_id::text ILIKE '%' || $1 || '%')
                AND ($2::text IS NULL OR r.role=$2)
              ORDER BY c.email LIMIT $3",
        )
        .bind(search.trim())
        .bind(clean_filter(role))
        .bind(limit.clamp(1, 50))
        .fetch_all(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?;
        rows.into_iter()
            .map(|row| {
                Ok(json!({
                    "user_id":row.try_get::<Uuid,_>("user_id").map_err(InstitutionError::Database)?,
                    "email":row.try_get::<String,_>("email").map_err(InstitutionError::Database)?,
                    "role":row.try_get::<String,_>("role").map_err(InstitutionError::Database)?,
                }))
            })
            .collect()
    }

    pub async fn manual_link_identity(
        &self,
        actor: UserId,
        external_type: &str,
        external_id: &str,
        user_id: Uuid,
    ) -> Result<(), InstitutionError> {
        let kind = external_type.trim().to_ascii_uppercase();
        let (people_table, id_column, links_table, required_role) = identity_table(&kind)?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        let role: Option<String> =
            sqlx::query_scalar("SELECT role FROM latex_core.global_user_roles WHERE user_id=$1")
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(InstitutionError::Database)?;
        if required_role.is_some_and(|required| role.as_deref() != Some(required)) {
            return Err(InstitutionError::Conflict(format!(
                "{kind} identity requires a V2 {} account",
                required_role.unwrap_or_default()
            )));
        }
        let user_linked_elsewhere: bool = sqlx::query_scalar(
            r"SELECT EXISTS(
                 SELECT 1 FROM vcap.student_user_links WHERE user_id=$1
                 UNION ALL SELECT 1 FROM vcap.faculty_user_links WHERE user_id=$1
                 UNION ALL SELECT 1 FROM vcap.admin_user_links WHERE user_id=$1
               )",
        )
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        if user_linked_elsewhere {
            return Err(InstitutionError::Conflict(
                "V2 user is already linked to an institutional identity".into(),
            ));
        }
        let person_exists =
            format!("SELECT EXISTS(SELECT 1 FROM vcap.{people_table} WHERE {id_column}=$1)");
        let exists: bool = sqlx::query_scalar(&person_exists)
            .bind(external_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?;
        if !exists {
            return Err(InstitutionError::NotFound);
        }
        let statement = format!(
            "INSERT INTO vcap.{links_table} ({id_column},user_id,match_method,status,linked_at) VALUES ($1,$2,'MANUAL','LINKED',statement_timestamp()) ON CONFLICT({id_column}) DO UPDATE SET user_id=EXCLUDED.user_id,match_method='MANUAL',status='LINKED',linked_at=statement_timestamp()"
        );
        sqlx::query(&statement)
            .bind(external_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|error| {
                if error
                    .as_database_error()
                    .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
                {
                    InstitutionError::Conflict(
                        "institutional identity or V2 user is already linked".into(),
                    )
                } else {
                    InstitutionError::Database(error)
                }
            })?;
        audit_tx(&mut tx, actor, "institution.identity.manual_link", links_table, Uuid::nil(), json!({
            "external_type":kind,"external_id":external_id,"user_id":user_id,"match_method":"MANUAL"
        })).await?;
        tx.commit().await.map_err(InstitutionError::Database)
    }

    pub async fn unlink_identity(
        &self,
        actor: UserId,
        external_type: &str,
        external_id: &str,
    ) -> Result<(), InstitutionError> {
        let kind = external_type.trim().to_ascii_uppercase();
        let (_, id_column, links_table, _) = identity_table(&kind)?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        let used_by_active_team: bool = match kind.as_str() {
            "STUDENT" => sqlx::query_scalar(
                r"SELECT EXISTS(SELECT 1 FROM vcap.paper_assignment_students a
                   JOIN latex_core.external_paper_team_links x USING(external_team_key)
                   JOIN latex_core.paper_teams t ON t.id=x.paper_team_id
                   WHERE a.student_reg_no=$1 AND t.status<>'archived')",
            )
            .bind(external_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?,
            "FACULTY" => sqlx::query_scalar(
                r"SELECT EXISTS(SELECT 1 FROM vcap.paper_assignment_mentors a
                   JOIN latex_core.external_paper_team_links x USING(external_team_key)
                   JOIN latex_core.paper_teams t ON t.id=x.paper_team_id
                   WHERE a.faculty_id=$1 AND t.status<>'archived')",
            )
            .bind(external_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?,
            "ADMIN" => false,
            _ => unreachable!(),
        };
        if used_by_active_team {
            return Err(InstitutionError::Conflict(
                "identity link is used by an active imported Paper Team and cannot be removed"
                    .into(),
            ));
        }
        let statement = format!(
            "UPDATE vcap.{links_table} SET user_id=NULL,match_method='MANUAL',status='UNLINKED',linked_at=NULL WHERE {id_column}=$1"
        );
        let affected = sqlx::query(&statement)
            .bind(external_id)
            .execute(&mut *tx)
            .await
            .map_err(InstitutionError::Database)?
            .rows_affected();
        if affected == 0 {
            return Err(InstitutionError::NotFound);
        }
        audit_tx(
            &mut tx,
            actor,
            "institution.identity.manual_unlink",
            links_table,
            Uuid::nil(),
            json!({
                "external_type":kind,"external_id":external_id
            }),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)
    }

    pub async fn audit_bulk_lifecycle(
        &self,
        actor: UserId,
        requested_status: &str,
        succeeded: usize,
        failed: usize,
    ) -> Result<(), InstitutionError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        audit_tx(
            &mut tx,
            actor,
            "institution.team.bulk_lifecycle",
            "paper_team",
            Uuid::nil(),
            json!({
                "requested_status":requested_status,"succeeded":succeeded,"failed":failed
            }),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)
    }

    pub async fn audit_template_preview(
        &self,
        actor: UserId,
        paper_id: Uuid,
        new_template_id: Uuid,
        conflicts: usize,
    ) -> Result<(), InstitutionError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        audit_tx(
            &mut tx,
            actor,
            "institution.team.template_previewed",
            "paper_team",
            paper_id,
            json!({
                "new_template_id":new_template_id,"blocking_conflicts":conflicts
            }),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)
    }

    pub async fn paper_team_admin_summary(
        &self,
        paper_id: Uuid,
    ) -> Result<Value, InstitutionError> {
        let row = sqlx::query(
            r"SELECT resolution.dominant_programme_code,resolution.resolution_method,
                     link.external_team_key,link.source_import_job_id,job.applied_at::text AS last_imported_at,
                     COALESCE(review.status,'NONE') AS review_state,
                     (SELECT count(*) FROM latex_core.paper_files f WHERE f.workspace_id=t.workspace_id AND NOT f.tombstoned) AS file_count,
                     build.id AS current_build_id,build.status AS current_build_status
              FROM latex_core.paper_teams t
              LEFT JOIN latex_core.paper_template_resolutions resolution ON resolution.paper_team_id=t.id
              LEFT JOIN latex_core.external_paper_team_links link ON link.paper_team_id=t.id
              LEFT JOIN latex_core.institution_import_jobs job ON job.id=link.source_import_job_id
              LEFT JOIN LATERAL (SELECT r.status FROM latex_core.review_rounds r WHERE r.paper_id=t.id ORDER BY r.opened_at DESC,r.id DESC LIMIT 1) review ON TRUE
              LEFT JOIN latex_core.v2_paper_build_state bs ON bs.paper_id=t.id
              LEFT JOIN latex_core.v2_paper_builds build ON build.id=COALESCE(bs.active_build_id,bs.current_build_id)
              WHERE t.id=$1",
        )
        .bind(paper_id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(InstitutionError::Database)?
        .ok_or(InstitutionError::NotFound)?;
        Ok(json!({
            "dominant_programme_code":row.try_get::<Option<String>,_>("dominant_programme_code").map_err(InstitutionError::Database)?,
            "resolution_method":row.try_get::<Option<String>,_>("resolution_method").map_err(InstitutionError::Database)?,
            "external_team_key":row.try_get::<Option<String>,_>("external_team_key").map_err(InstitutionError::Database)?,
            "source_import_job_id":row.try_get::<Option<Uuid>,_>("source_import_job_id").map_err(InstitutionError::Database)?,
            "last_imported_at":row.try_get::<Option<String>,_>("last_imported_at").map_err(InstitutionError::Database)?,
            "review_state":row.try_get::<String,_>("review_state").map_err(InstitutionError::Database)?,
            "file_count":row.try_get::<i64,_>("file_count").map_err(InstitutionError::Database)?,
            "current_build":{
                "id":row.try_get::<Option<Uuid>,_>("current_build_id").map_err(InstitutionError::Database)?,
                "status":row.try_get::<Option<String>,_>("current_build_status").map_err(InstitutionError::Database)?,
            }
        }))
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
        sqlx::query(
            r"UPDATE latex_core.institution_import_batches batch SET status='PARTIAL',
                 error_rows=(SELECT count(*) FROM latex_core.institution_import_rows row
                     JOIN latex_core.institution_import_jobs sibling ON sibling.id=row.job_id
                     WHERE sibling.batch_id=batch.id AND row.status IN ('ERROR','UNRESOLVED'))
               WHERE batch.id=(SELECT batch_id FROM latex_core.institution_import_jobs WHERE id=$1)",
        )
        .bind(job_id)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        tx.commit().await.map_err(InstitutionError::Database)
    }

    pub async fn mark_team_resolved(
        &self,
        actor: UserId,
        job_id: Uuid,
        external_team_key: &str,
    ) -> Result<(), InstitutionError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(InstitutionError::Database)?;
        sqlx::query("UPDATE latex_core.institution_import_rows SET status=CASE WHEN action='SKIP' THEN 'SKIPPED' ELSE 'APPLIED' END,error_code=NULL,error_message=NULL WHERE job_id=$1 AND (natural_key->>'external_team_key')=$2 AND status='UNRESOLVED'")
            .bind(job_id).bind(external_team_key).execute(&mut *tx).await.map_err(InstitutionError::Database)?;
        let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.institution_import_rows WHERE job_id=$1 AND status IN ('ERROR','UNRESOLVED')")
            .bind(job_id).fetch_one(&mut *tx).await.map_err(InstitutionError::Database)?;
        sqlx::query("UPDATE latex_core.institution_import_jobs SET error_rows=$2,status=CASE WHEN $2=0 THEN 'APPLIED' ELSE 'PARTIAL' END WHERE id=$1")
            .bind(job_id).bind(remaining).execute(&mut *tx).await.map_err(InstitutionError::Database)?;
        sqlx::query(
            r"UPDATE latex_core.institution_import_batches batch SET
                 error_rows=(SELECT count(*) FROM latex_core.institution_import_rows row
                     JOIN latex_core.institution_import_jobs sibling ON sibling.id=row.job_id
                     WHERE sibling.batch_id=batch.id AND row.status IN ('ERROR','UNRESOLVED')),
                 status=CASE WHEN NOT EXISTS(SELECT 1 FROM latex_core.institution_import_rows row
                     JOIN latex_core.institution_import_jobs sibling ON sibling.id=row.job_id
                     WHERE sibling.batch_id=batch.id AND row.status IN ('ERROR','UNRESOLVED')) THEN 'APPLIED' ELSE 'PARTIAL' END
               WHERE batch.id=(SELECT batch_id FROM latex_core.institution_import_jobs WHERE id=$1)",
        )
        .bind(job_id)
        .execute(&mut *tx)
        .await
        .map_err(InstitutionError::Database)?;
        audit_tx(
            &mut tx,
            actor,
            "institution.team.materialization_retried",
            "institution_import_job",
            job_id,
            json!({"external_team_key":external_team_key,"resolved":true}),
        )
        .await?;
        tx.commit().await.map_err(InstitutionError::Database)
    }

    pub async fn pending_team_plans(
        &self,
        job_id: Uuid,
    ) -> Result<Vec<ImportedTeamPlan>, InstitutionError> {
        let groups = sqlx::query(
            "SELECT g.external_team_key,g.team_name,link.paper_team_id FROM vcap.paper_assignment_groups g \
             LEFT JOIN latex_core.external_paper_team_links link USING(external_team_key) \
             WHERE EXISTS (SELECT 1 FROM latex_core.institution_import_rows r \
                 JOIN latex_core.institution_import_jobs source_job ON source_job.id=r.job_id \
                 WHERE (r.job_id=$1 OR source_job.batch_id=(SELECT batch_id FROM latex_core.institution_import_jobs WHERE id=$1)) \
                 AND r.source_table_or_sheet IN ('paper_teams','paper_team_writers','paper_team_mentors') \
                 AND (r.natural_key->>'external_team_key')=g.external_team_key) \
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
                     COALESCE((SELECT jsonb_agg(jsonb_build_object('user_id',m.user_id,'email',c.email,'writer_order',m.writer_order) ORDER BY m.writer_order,c.email) FROM latex_core.paper_team_members m JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role='writer' JOIN latex_core.user_credentials c ON c.user_id=m.user_id WHERE m.paper_team_id=t.id),'[]'::jsonb) AS writers,
                     (SELECT jsonb_build_object('user_id',m.user_id,'email',c.email) FROM latex_core.paper_team_members m JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role='writer' JOIN latex_core.user_credentials c ON c.user_id=m.user_id WHERE m.paper_team_id=t.id AND m.is_leader LIMIT 1) AS leader,
                     COALESCE((SELECT jsonb_agg(jsonb_build_object('user_id',m.user_id,'email',c.email) ORDER BY c.email) FROM latex_core.paper_team_members m JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role='mentor' JOIN latex_core.user_credentials c ON c.user_id=m.user_id WHERE m.paper_team_id=t.id),'[]'::jsonb) AS mentors,
                     resolution.dominant_programme_code,resolution.resolution_method,
                     pin.template_id,template.name AS template_name,
                     COALESCE(review.status,'NONE') AS review_state,
                     link.external_team_key,link.source_import_job_id,job.applied_at::text AS last_imported_at,
                     CASE WHEN link.external_team_key IS NULL THEN 'manual' ELSE 'imported' END AS source
              FROM latex_core.paper_teams t
              LEFT JOIN latex_core.paper_template_resolutions resolution ON resolution.paper_team_id=t.id
              LEFT JOIN latex_core.paper_template_pins pin ON pin.paper_id=t.id
              LEFT JOIN latex_core.templates template ON template.id=pin.template_id
              LEFT JOIN latex_core.external_paper_team_links link ON link.paper_team_id=t.id
              LEFT JOIN latex_core.institution_import_jobs job ON job.id=link.source_import_job_id
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
                "writers": row.try_get::<Value,_>("writers").map_err(InstitutionError::Database)?,
                "leader": row.try_get::<Option<Value>,_>("leader").map_err(InstitutionError::Database)?,
                "mentors": row.try_get::<Value,_>("mentors").map_err(InstitutionError::Database)?,
                "dominant_programme_code": row.try_get::<Option<String>,_>("dominant_programme_code").map_err(InstitutionError::Database)?,
                "template": {"id":row.try_get::<Option<Uuid>,_>("template_id").map_err(InstitutionError::Database)?,"name":row.try_get::<Option<String>,_>("template_name").map_err(InstitutionError::Database)?},
                "template_resolution_method": row.try_get::<Option<String>,_>("resolution_method").map_err(InstitutionError::Database)?,
                "review_state": row.try_get::<String,_>("review_state").map_err(InstitutionError::Database)?,
                "updated_at": row.try_get::<String,_>("updated_at").map_err(InstitutionError::Database)?,
                "source": row.try_get::<String,_>("source").map_err(InstitutionError::Database)?,
                "external_team_key": row.try_get::<Option<String>,_>("external_team_key").map_err(InstitutionError::Database)?,
                "source_import_job_id": row.try_get::<Option<Uuid>,_>("source_import_job_id").map_err(InstitutionError::Database)?,
                "last_imported_at": row.try_get::<Option<String>,_>("last_imported_at").map_err(InstitutionError::Database)?,
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
                  SELECT g.external_team_key,g.team_name,g.status,g.academic_year,g.semester,
                         max(job.created_at)::text AS updated_at,
                         count(DISTINCT writer.student_reg_no) AS writer_count,
                         (array_agg(import_row.error_code ORDER BY import_row.row_number) FILTER (WHERE import_row.error_code IS NOT NULL))[1] AS error_code,
                         (array_agg(import_row.error_message ORDER BY import_row.row_number) FILTER (WHERE import_row.error_message IS NOT NULL))[1] AS error_message,
                         (array_agg(job.id ORDER BY job.created_at DESC))[1] AS source_import_job_id,
                         ARRAY(SELECT assigned.student_reg_no FROM vcap.paper_assignment_students assigned LEFT JOIN vcap.student_user_links identity ON identity.reg_no=assigned.student_reg_no AND identity.status='LINKED' WHERE assigned.external_team_key=g.external_team_key AND identity.user_id IS NULL ORDER BY assigned.writer_order) AS unresolved_student_ids,
                         ARRAY(SELECT assigned.faculty_id FROM vcap.paper_assignment_mentors assigned LEFT JOIN vcap.faculty_user_links identity ON identity.faculty_id=assigned.faculty_id AND identity.status='LINKED' WHERE assigned.external_team_key=g.external_team_key AND identity.user_id IS NULL ORDER BY assigned.faculty_id) AS unresolved_faculty_ids,
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
                  GROUP BY g.external_team_key,g.team_name,g.status,g.academic_year,g.semester
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
                "writers": [],
                "leader": null,
                "mentors": [],
                "dominant_programme_code": null,
                "template": {"id":null,"name":null},
                "template_resolution_method": null,
                "review_state": "NONE",
                "updated_at": row.try_get::<String,_>("updated_at").map_err(InstitutionError::Database)?,
                "source": "imported",
                "external_team_key": row.try_get::<String,_>("external_team_key").map_err(InstitutionError::Database)?,
                "academic_year": row.try_get::<Option<String>,_>("academic_year").map_err(InstitutionError::Database)?,
                "semester": row.try_get::<Option<String>,_>("semester").map_err(InstitutionError::Database)?,
                "error_code": row.try_get::<Option<String>,_>("error_code").map_err(InstitutionError::Database)?,
                "error_message": row.try_get::<Option<String>,_>("error_message").map_err(InstitutionError::Database)?,
                "unresolved_student_ids": row.try_get::<Vec<String>,_>("unresolved_student_ids").map_err(InstitutionError::Database)?,
                "unresolved_faculty_ids": row.try_get::<Vec<String>,_>("unresolved_faculty_ids").map_err(InstitutionError::Database)?,
                "source_import_job_id": row.try_get::<Uuid,_>("source_import_job_id").map_err(InstitutionError::Database)?,
                "last_imported_at": row.try_get::<String,_>("updated_at").map_err(InstitutionError::Database)?,
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
        let mut existing = HashMap::<String, HashMap<String, Value>>::new();
        for table in INSTITUTION_TABLES {
            if rows
                .iter()
                .any(|row| row.table == table && row.error.is_none())
            {
                existing.insert(table.to_owned(), self.existing_records(table).await?);
            }
        }
        for row in rows {
            if row.error.is_some() || matches!(action_for(row), "SKIP" | "DELETE") {
                continue;
            }
            let existing_payload = existing
                .get(&row.table)
                .and_then(|records| records.get(&key_token(&row.natural_key)))
                .cloned();
            let present = existing_payload.is_some();
            row.existing_payload = existing_payload;
            if !present && matches!(mode, ImportMode::UpdateOnly | ImportMode::DeleteOnly) {
                row.error = Some((
                    "NOT_FOUND",
                    format!(
                        "{} was not found; check the canonical key and try again",
                        friendly_dataset(&row.table)
                    ),
                ));
                continue;
            }
            row.payload
                .as_object_mut()
                .expect("validated payload object")
                .insert(
                    "__action".into(),
                    Value::String(
                        match (present, mode) {
                            (
                                false,
                                ImportMode::ValidateOnly | ImportMode::Merge | ImportMode::AddOnly,
                            ) => "INSERT",
                            (true, ImportMode::Merge | ImportMode::UpdateOnly) => "UPDATE",
                            (true, ImportMode::DeleteOnly) => "DELETE",
                            (true, ImportMode::ValidateOnly | ImportMode::AddOnly) => "SKIP",
                            (false, ImportMode::UpdateOnly | ImportMode::DeleteOnly) => "INVALID",
                        }
                        .into(),
                    ),
                );
        }
        Ok(())
    }

    async fn existing_records(
        &self,
        table: &str,
    ) -> Result<HashMap<String, Value>, InstitutionError> {
        let query = match table {
            "departments" => {
                "SELECT jsonb_build_object('department_id',department_id::text) AS natural_key,to_jsonb(record) AS payload FROM vcap.departments record"
            }
            "admins" => {
                "SELECT jsonb_build_object('admin_id',admin_id) AS natural_key,to_jsonb(record) AS payload FROM vcap.admins record"
            }
            "faculty" => {
                "SELECT jsonb_build_object('faculty_id',faculty_id) AS natural_key,to_jsonb(record) AS payload FROM vcap.faculty record"
            }
            "programmes" => {
                "SELECT jsonb_build_object('programme_code',programme_code) AS natural_key,to_jsonb(record) AS payload FROM vcap.programmes record"
            }
            "schools" => {
                "SELECT jsonb_build_object('school_id',school_id) AS natural_key,to_jsonb(record) AS payload FROM vcap.schools record"
            }
            "students" => {
                "SELECT jsonb_build_object('reg_no',reg_no) AS natural_key,to_jsonb(record) AS payload FROM vcap.students record"
            }
            "student_course_registrations" => {
                "SELECT jsonb_build_object('student_reg_no',student_reg_no,'course_id',course_id,'academic_year',academic_year,'semester',semester) AS natural_key,to_jsonb(record) AS payload FROM vcap.student_course_registrations record"
            }
            "faculty_guide_capacity" => {
                "SELECT jsonb_build_object('capacity_id',capacity_id::text) AS natural_key,to_jsonb(record) AS payload FROM vcap.faculty_guide_capacity record"
            }
            "department_roles" => {
                "SELECT jsonb_build_object('id',id::text) AS natural_key,to_jsonb(record) AS payload FROM vcap.department_roles record"
            }
            "faculty_roles" => {
                "SELECT jsonb_build_object('role_id',role_id::text) AS natural_key,to_jsonb(record) AS payload FROM vcap.faculty_roles record"
            }
            "paper_teams" => {
                "SELECT jsonb_build_object('external_team_key',external_team_key) AS natural_key,to_jsonb(record) AS payload FROM vcap.paper_assignment_groups record"
            }
            "paper_team_writers" => {
                "SELECT jsonb_build_object('external_team_key',external_team_key,'student_reg_no',student_reg_no) AS natural_key,to_jsonb(record) AS payload FROM vcap.paper_assignment_students record"
            }
            "paper_team_mentors" => {
                "SELECT jsonb_build_object('external_team_key',external_team_key,'faculty_id',faculty_id) AS natural_key,to_jsonb(record) AS payload FROM vcap.paper_assignment_mentors record"
            }
            _ => {
                return Err(InstitutionError::InvalidInput(
                    "unknown target table".into(),
                ));
            }
        };
        let values = sqlx::query(query)
            .fetch_all(self.database.pool())
            .await
            .map_err(InstitutionError::Database)?;
        values
            .into_iter()
            .map(|row| {
                let natural_key: Value = row
                    .try_get("natural_key")
                    .map_err(InstitutionError::Database)?;
                let payload: Value = row.try_get("payload").map_err(InstitutionError::Database)?;
                Ok((key_token(&natural_key), payload))
            })
            .collect()
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

    async fn validate_delete_dependencies(
        &self,
        rows: &mut [SourceRow],
    ) -> Result<(), InstitutionError> {
        for row in rows
            .iter_mut()
            .filter(|row| row.error.is_none() && action_for(row) == "DELETE")
        {
            let payload = row.payload.as_object().expect("validated payload object");
            let (query, key) = match row.table.as_str() {
                "students" => (
                    r"SELECT 'Course registration ' || course_id FROM vcap.student_course_registrations WHERE student_reg_no=$1
                      UNION ALL SELECT 'Paper Team ' || external_team_key || ' — Writer' FROM vcap.paper_assignment_students WHERE student_reg_no=$1",
                    text_value(payload, "reg_no"),
                ),
                "faculty" => (
                    r"SELECT 'Guide capacity ' || capacity_id::text FROM vcap.faculty_guide_capacity WHERE faculty_id=$1
                      UNION ALL SELECT 'Department role ' || id::text FROM vcap.department_roles WHERE faculty_id=$1
                      UNION ALL SELECT 'Faculty role ' || role_id::text FROM vcap.faculty_roles WHERE faculty_id=$1
                      UNION ALL SELECT 'Paper Team ' || external_team_key || ' — Mentor' FROM vcap.paper_assignment_mentors WHERE faculty_id=$1
                      UNION ALL SELECT 'Programme ' || programme_code || ' — HOD' FROM vcap.programmes WHERE hod_id=$1",
                    text_value(payload, "faculty_id"),
                ),
                "programmes" => (
                    r"SELECT 'Student ' || reg_no FROM vcap.students WHERE programme_code=$1
                      UNION ALL SELECT 'Faculty role ' || role_id::text FROM vcap.faculty_roles WHERE programme_code=$1
                      UNION ALL SELECT 'Programme template default' FROM latex_core.programme_template_defaults WHERE programme_code=$1
                      UNION ALL SELECT 'Existing Team template resolution' FROM latex_core.paper_template_resolutions WHERE dominant_programme_code=$1",
                    text_value(payload, "programme_code"),
                ),
                "departments" => (
                    r"SELECT 'Faculty ' || faculty_id FROM vcap.faculty WHERE dept_id::text=$1
                      UNION ALL SELECT 'Department role ' || id::text FROM vcap.department_roles WHERE dept_id=$1
                      UNION ALL SELECT 'Faculty role ' || role_id::text FROM vcap.faculty_roles WHERE department_id::text=$1",
                    text_value(payload, "department_id"),
                ),
                "schools" => (
                    "SELECT 'Faculty role ' || role_id::text FROM vcap.faculty_roles WHERE school_id=$1",
                    text_value(payload, "school_id"),
                ),
                "paper_teams" => (
                    r"SELECT 'Writer ' || student_reg_no FROM vcap.paper_assignment_students WHERE external_team_key=$1
                      UNION ALL SELECT 'Mentor ' || faculty_id FROM vcap.paper_assignment_mentors WHERE external_team_key=$1
                      UNION ALL SELECT 'Materialized LaTeX Core Paper Team ' || paper_team_id::text FROM latex_core.external_paper_team_links WHERE external_team_key=$1",
                    text_value(payload, "external_team_key"),
                ),
                "paper_team_writers" | "paper_team_mentors" => (
                    "SELECT 'Materialized LaTeX Core Paper Team ' || paper_team_id::text FROM latex_core.external_paper_team_links WHERE external_team_key=$1",
                    text_value(payload, "external_team_key"),
                ),
                _ => continue,
            };
            let Some(key) = key else {
                continue;
            };
            let dependencies: Vec<String> = sqlx::query_scalar(query)
                .bind(key)
                .fetch_all(self.database.pool())
                .await
                .map_err(InstitutionError::Database)?;
            if !dependencies.is_empty() {
                row.error = Some((
                    "DELETE_BLOCKED_DEPENDENCY",
                    format!(
                        "Cannot delete this {}. Used by: {}. Remove or archive those relationships explicitly first.",
                        friendly_dataset(&row.table),
                        dependencies.join("; ")
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn friendly_dataset(table: &str) -> String {
    match table {
        "paper_teams" => "Paper assignment".into(),
        "paper_team_writers" => "Paper assignment Writer".into(),
        "paper_team_mentors" => "Paper assignment Mentor".into(),
        value => value
            .trim_end_matches('s')
            .replace('_', " ")
            .split_whitespace()
            .map(|word| {
                let mut characters = word.chars();
                characters.next().map_or_else(String::new, |first| {
                    first.to_uppercase().collect::<String>() + characters.as_str()
                })
            })
            .collect::<Vec<_>>()
            .join(" "),
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
    let normalized = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    INSTITUTION_TABLES
        .into_iter()
        .find(|table| normalized == *table)
        .or_else(|| {
            INSTITUTION_TABLES
                .into_iter()
                .filter(|table| {
                    normalized.starts_with(&format!("{table}_"))
                        || normalized.ends_with(&format!("_{table}"))
                        || normalized.contains(&format!("_{table}_"))
                })
                .max_by_key(|table| table.len())
        })
}

fn infer_csv_table_from_headers(
    headers: &StringRecord,
    mode: ImportMode,
) -> Result<&'static str, InstitutionError> {
    let candidates = INSTITUTION_TABLES
        .into_iter()
        .filter(|table| {
            validate_headers(table, headers.iter(), ImportLimits::default(), mode).is_ok()
        })
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [table] => Ok(*table),
        [] => Err(InstitutionError::InvalidInput(
            "CSV columns do not match a supported dataset; check the header row".into(),
        )),
        values => Err(InstitutionError::InvalidInput(format!(
            "What data is this? Matching datasets: {}",
            values.join(", ")
        ))),
    }
}

fn parse_csv(
    filename: &str,
    target: Option<&str>,
    bytes: &[u8],
    limits: ImportLimits,
    mode: ImportMode,
    upload_index: usize,
) -> Result<Vec<SourceRow>, InstitutionError> {
    let mut reader = csv::ReaderBuilder::new().flexible(false).from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|error| InstitutionError::InvalidInput(format!("malformed CSV header: {error}")))?
        .clone();
    let table = match target.or_else(|| infer_csv_table(filename)) {
        Some(table) if INSTITUTION_TABLES.contains(&table) => table,
        Some(_) => {
            return Err(InstitutionError::InvalidInput(
                "unknown CSV target table".into(),
            ));
        }
        None => infer_csv_table_from_headers(&headers, mode)?,
    };
    validate_headers(table, headers.iter(), limits, mode)?;
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
            mode,
            upload_index,
        )?);
    }
    Ok(rows)
}

fn parse_xlsx(
    bytes: &[u8],
    limits: ImportLimits,
    mode: ImportMode,
    upload_index: usize,
) -> Result<Vec<SourceRow>, InstitutionError> {
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
        validate_headers(
            &name,
            header_values.iter().map(String::as_str),
            limits,
            mode,
        )?;
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
                mode,
                upload_index,
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
    mode: ImportMode,
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
    let required = if matches!(mode, ImportMode::UpdateOnly | ImportMode::DeleteOnly) {
        spec(table).keys
    } else {
        spec(table).required
    };
    for required in required {
        if !unique.contains(required) {
            return Err(InstitutionError::InvalidInput(format!(
                "{table}: missing required column {required}"
            )));
        }
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "bounded parser context is explicit at the row validation boundary"
)]
fn source_row(
    table: &str,
    row_number: i64,
    headers: &StringRecord,
    record: &StringRecord,
    formula_error: Option<&str>,
    limits: ImportLimits,
    mode: ImportMode,
    upload_index: usize,
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
        let required = if matches!(mode, ImportMode::UpdateOnly | ImportMode::DeleteOnly) {
            table_spec.keys
        } else {
            table_spec.required
        };
        for field in required {
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
        upload_index,
        table: table.to_owned(),
        row_number,
        natural_key,
        payload: Value::Object(payload),
        existing_payload: None,
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
    mode: ImportMode,
    payloads: Value,
) -> Result<(), InstitutionError> {
    if mode == ImportMode::DeleteOnly {
        let link_delete = match table {
            "students" => Some(
                "DELETE FROM vcap.student_user_links WHERE reg_no IN (SELECT payload->>'reg_no' FROM jsonb_array_elements($1) item(payload))",
            ),
            "faculty" => Some(
                "DELETE FROM vcap.faculty_user_links WHERE faculty_id IN (SELECT payload->>'faculty_id' FROM jsonb_array_elements($1) item(payload))",
            ),
            "admins" => Some(
                "DELETE FROM vcap.admin_user_links WHERE admin_id IN (SELECT payload->>'admin_id' FROM jsonb_array_elements($1) item(payload))",
            ),
            _ => None,
        };
        if let Some(statement) = link_delete {
            sqlx::query(statement)
                .bind(&payloads)
                .execute(&mut **tx)
                .await
                .map_err(InstitutionError::Database)?;
        }
    }
    let query = match mode {
        ImportMode::Merge => apply_query(table, true),
        ImportMode::ValidateOnly | ImportMode::AddOnly => apply_query(table, false),
        ImportMode::UpdateOnly => update_query(table),
        ImportMode::DeleteOnly => delete_query(table),
    };
    sqlx::query(query)
        .bind(&payloads)
        .execute(&mut **tx)
        .await
        .map_err(InstitutionError::Database)?;
    if table == "department_roles" && mode != ImportMode::DeleteOnly {
        sqlx::query("SELECT setval(pg_get_serial_sequence('vcap.department_roles','id'),GREATEST((SELECT COALESCE(max(id),1) FROM vcap.department_roles),1),TRUE)").execute(&mut **tx).await.map_err(InstitutionError::Database)?;
    }
    Ok(())
}

fn update_query(table: &str) -> &'static str {
    match table {
        "departments" => {
            "UPDATE vcap.departments record SET department_id=record.department_id FROM jsonb_array_elements($1) item(payload) WHERE record.department_id::text=item.payload->>'department_id'"
        }
        "admins" => {
            "UPDATE vcap.admins record SET email=CASE WHEN item.payload ? 'email' THEN item.payload->>'email' ELSE record.email END,name=CASE WHEN item.payload ? 'name' THEN item.payload->>'name' ELSE record.name END,pfp=CASE WHEN item.payload ? 'pfp' THEN item.payload->>'pfp' ELSE record.pfp END FROM jsonb_array_elements($1) item(payload) WHERE record.admin_id=item.payload->>'admin_id'"
        }
        "faculty" => {
            "UPDATE vcap.faculty record SET name=CASE WHEN item.payload ? 'name' THEN item.payload->>'name' ELSE record.name END,email=CASE WHEN item.payload ? 'email' THEN item.payload->>'email' ELSE record.email END,dept_id=CASE WHEN item.payload ? 'dept_id' THEN (item.payload->>'dept_id')::uuid ELSE record.dept_id END,honorific=CASE WHEN item.payload ? 'honorific' THEN item.payload->>'honorific' ELSE record.honorific END,designation=CASE WHEN item.payload ? 'designation' THEN item.payload->>'designation' ELSE record.designation END,status=CASE WHEN item.payload ? 'status' THEN item.payload->>'status' ELSE record.status END FROM jsonb_array_elements($1) item(payload) WHERE record.faculty_id=item.payload->>'faculty_id'"
        }
        "programmes" => {
            "UPDATE vcap.programmes record SET hod_id=CASE WHEN item.payload ? 'hod_id' THEN item.payload->>'hod_id' ELSE record.hod_id END FROM jsonb_array_elements($1) item(payload) WHERE record.programme_code=item.payload->>'programme_code'"
        }
        "schools" => {
            "UPDATE vcap.schools record SET school_id=record.school_id FROM jsonb_array_elements($1) item(payload) WHERE record.school_id=item.payload->>'school_id'"
        }
        "students" => {
            "UPDATE vcap.students record SET name=CASE WHEN item.payload ? 'name' THEN item.payload->>'name' ELSE record.name END,email=CASE WHEN item.payload ? 'email' THEN item.payload->>'email' ELSE record.email END,programme_code=CASE WHEN item.payload ? 'programme_code' THEN item.payload->>'programme_code' ELSE record.programme_code END FROM jsonb_array_elements($1) item(payload) WHERE record.reg_no=item.payload->>'reg_no'"
        }
        "student_course_registrations" => {
            "UPDATE vcap.student_course_registrations record SET registration_status=CASE WHEN item.payload ? 'registration_status' THEN item.payload->>'registration_status' ELSE record.registration_status END FROM jsonb_array_elements($1) item(payload) WHERE record.student_reg_no=item.payload->>'student_reg_no' AND record.course_id=item.payload->>'course_id' AND record.academic_year=item.payload->>'academic_year' AND record.semester=item.payload->>'semester'"
        }
        "faculty_guide_capacity" => {
            "UPDATE vcap.faculty_guide_capacity record SET faculty_id=CASE WHEN item.payload ? 'faculty_id' THEN item.payload->>'faculty_id' ELSE record.faculty_id END,academic_year=CASE WHEN item.payload ? 'academic_year' THEN item.payload->>'academic_year' ELSE record.academic_year END,ug_max_projects=CASE WHEN item.payload ? 'ug_max_projects' THEN (item.payload->>'ug_max_projects')::int ELSE record.ug_max_projects END,pg_max_projects=CASE WHEN item.payload ? 'pg_max_projects' THEN (item.payload->>'pg_max_projects')::int ELSE record.pg_max_projects END,integrated_pg_max_projects=CASE WHEN item.payload ? 'integrated_pg_max_projects' THEN (item.payload->>'integrated_pg_max_projects')::int ELSE record.integrated_pg_max_projects END,status=CASE WHEN item.payload ? 'status' THEN item.payload->>'status' ELSE record.status END FROM jsonb_array_elements($1) item(payload) WHERE record.capacity_id::text=item.payload->>'capacity_id'"
        }
        "department_roles" => {
            "UPDATE vcap.department_roles record SET dept_id=CASE WHEN item.payload ? 'dept_id' THEN item.payload->>'dept_id' ELSE record.dept_id END,role_type=CASE WHEN item.payload ? 'role_type' THEN item.payload->>'role_type' ELSE record.role_type END,faculty_id=CASE WHEN item.payload ? 'faculty_id' THEN item.payload->>'faculty_id' ELSE record.faculty_id END FROM jsonb_array_elements($1) item(payload) WHERE record.id::text=item.payload->>'id'"
        }
        "faculty_roles" => {
            "UPDATE vcap.faculty_roles record SET faculty_id=CASE WHEN item.payload ? 'faculty_id' THEN item.payload->>'faculty_id' ELSE record.faculty_id END,role_type=CASE WHEN item.payload ? 'role_type' THEN item.payload->>'role_type' ELSE record.role_type END,school_id=CASE WHEN item.payload ? 'school_id' THEN item.payload->>'school_id' ELSE record.school_id END,department_id=CASE WHEN item.payload ? 'department_id' THEN (item.payload->>'department_id')::uuid ELSE record.department_id END,programme_code=CASE WHEN item.payload ? 'programme_code' THEN item.payload->>'programme_code' ELSE record.programme_code END,status=CASE WHEN item.payload ? 'status' THEN item.payload->>'status' ELSE record.status END FROM jsonb_array_elements($1) item(payload) WHERE record.role_id::text=item.payload->>'role_id'"
        }
        "paper_teams" => {
            "UPDATE vcap.paper_assignment_groups record SET team_name=CASE WHEN item.payload ? 'team_name' THEN item.payload->>'team_name' ELSE record.team_name END,academic_year=CASE WHEN item.payload ? 'academic_year' THEN item.payload->>'academic_year' ELSE record.academic_year END,semester=CASE WHEN item.payload ? 'semester' THEN item.payload->>'semester' ELSE record.semester END,status=CASE WHEN item.payload ? 'status' THEN item.payload->>'status' ELSE record.status END FROM jsonb_array_elements($1) item(payload) WHERE record.external_team_key=item.payload->>'external_team_key'"
        }
        "paper_team_writers" => {
            "UPDATE vcap.paper_assignment_students record SET writer_order=CASE WHEN item.payload ? 'writer_order' THEN (item.payload->>'writer_order')::int ELSE record.writer_order END,is_leader=CASE WHEN item.payload ? 'is_leader' THEN (item.payload->>'is_leader')::boolean ELSE record.is_leader END FROM jsonb_array_elements($1) item(payload) WHERE record.external_team_key=item.payload->>'external_team_key' AND record.student_reg_no=item.payload->>'student_reg_no'"
        }
        "paper_team_mentors" => {
            "UPDATE vcap.paper_assignment_mentors record SET faculty_id=record.faculty_id FROM jsonb_array_elements($1) item(payload) WHERE record.external_team_key=item.payload->>'external_team_key' AND record.faculty_id=item.payload->>'faculty_id'"
        }
        _ => unreachable!("update table is fixed"),
    }
}

fn delete_query(table: &str) -> &'static str {
    match table {
        "departments" => {
            "DELETE FROM vcap.departments record USING jsonb_array_elements($1) item(payload) WHERE record.department_id::text=item.payload->>'department_id'"
        }
        "admins" => {
            "DELETE FROM vcap.admins record USING jsonb_array_elements($1) item(payload) WHERE record.admin_id=item.payload->>'admin_id'"
        }
        "faculty" => {
            "DELETE FROM vcap.faculty record USING jsonb_array_elements($1) item(payload) WHERE record.faculty_id=item.payload->>'faculty_id'"
        }
        "programmes" => {
            "DELETE FROM vcap.programmes record USING jsonb_array_elements($1) item(payload) WHERE record.programme_code=item.payload->>'programme_code'"
        }
        "schools" => {
            "DELETE FROM vcap.schools record USING jsonb_array_elements($1) item(payload) WHERE record.school_id=item.payload->>'school_id'"
        }
        "students" => {
            "DELETE FROM vcap.students record USING jsonb_array_elements($1) item(payload) WHERE record.reg_no=item.payload->>'reg_no'"
        }
        "student_course_registrations" => {
            "DELETE FROM vcap.student_course_registrations record USING jsonb_array_elements($1) item(payload) WHERE record.student_reg_no=item.payload->>'student_reg_no' AND record.course_id=item.payload->>'course_id' AND record.academic_year=item.payload->>'academic_year' AND record.semester=item.payload->>'semester'"
        }
        "faculty_guide_capacity" => {
            "DELETE FROM vcap.faculty_guide_capacity record USING jsonb_array_elements($1) item(payload) WHERE record.capacity_id::text=item.payload->>'capacity_id'"
        }
        "department_roles" => {
            "DELETE FROM vcap.department_roles record USING jsonb_array_elements($1) item(payload) WHERE record.id::text=item.payload->>'id'"
        }
        "faculty_roles" => {
            "DELETE FROM vcap.faculty_roles record USING jsonb_array_elements($1) item(payload) WHERE record.role_id::text=item.payload->>'role_id'"
        }
        "paper_teams" => {
            "DELETE FROM vcap.paper_assignment_groups record USING jsonb_array_elements($1) item(payload) WHERE record.external_team_key=item.payload->>'external_team_key'"
        }
        "paper_team_writers" => {
            "DELETE FROM vcap.paper_assignment_students record USING jsonb_array_elements($1) item(payload) WHERE record.external_team_key=item.payload->>'external_team_key' AND record.student_reg_no=item.payload->>'student_reg_no'"
        }
        "paper_team_mentors" => {
            "DELETE FROM vcap.paper_assignment_mentors record USING jsonb_array_elements($1) item(payload) WHERE record.external_team_key=item.payload->>'external_team_key' AND record.faculty_id=item.payload->>'faculty_id'"
        }
        _ => unreachable!("delete table is fixed"),
    }
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

fn decode_batch(row: PgRow) -> Result<InstitutionImportBatch, InstitutionError> {
    Ok(InstitutionImportBatch {
        id: row.try_get("id").map_err(InstitutionError::Database)?,
        operation: row
            .try_get("operation")
            .map_err(InstitutionError::Database)?,
        status: row.try_get("status").map_err(InstitutionError::Database)?,
        submitted_by_user_id: row
            .try_get("submitted_by_user_id")
            .map_err(InstitutionError::Database)?,
        total_files: row
            .try_get("total_files")
            .map_err(InstitutionError::Database)?,
        total_rows: row
            .try_get("total_rows")
            .map_err(InstitutionError::Database)?,
        added_rows: row
            .try_get("added_rows")
            .map_err(InstitutionError::Database)?,
        edited_rows: row
            .try_get("edited_rows")
            .map_err(InstitutionError::Database)?,
        deleted_rows: row
            .try_get("deleted_rows")
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

fn count_rows(
    rows: &[SourceRow],
    predicate: impl Fn(&SourceRow) -> bool,
) -> Result<i64, InstitutionError> {
    i64::try_from(rows.iter().filter(|row| predicate(row)).count())
        .map_err(|_| InstitutionError::InvalidInput("too many rows".into()))
}

fn friendly_suggestion(code: Option<&str>) -> Option<&'static str> {
    match code {
        Some("INVALID_FK") => {
            Some("Add the referenced parent to this batch or correct the identifier.")
        }
        Some("NOT_FOUND") => {
            Some("Check the canonical key; Edit and Delete never create missing records.")
        }
        Some("DELETE_BLOCKED_DEPENDENCY") => Some(
            "Remove or archive the listed relationship explicitly, then review the delete again.",
        ),
        Some("INVALID_EMAIL") => Some("Use a complete institutional email address."),
        Some("MISSING_VALUE") => Some("Fill in the named required field."),
        Some("DUPLICATE_KEY") => Some("Keep one row for this canonical key in the batch."),
        _ => None,
    }
}

fn page_limit(limit: i64) -> i64 {
    if limit == 0 { 50 } else { limit.clamp(1, 100) }
}

fn clean_filter(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn identity_table(
    external_type: &str,
) -> Result<
    (
        &'static str,
        &'static str,
        &'static str,
        Option<&'static str>,
    ),
    InstitutionError,
> {
    match external_type {
        "STUDENT" => Ok(("students", "reg_no", "student_user_links", Some("writer"))),
        "FACULTY" => Ok((
            "faculty",
            "faculty_id",
            "faculty_user_links",
            Some("mentor"),
        )),
        "ADMIN" => Ok(("admins", "admin_id", "admin_user_links", None)),
        _ => Err(InstitutionError::InvalidInput(
            "external type must be STUDENT, FACULTY, or ADMIN".into(),
        )),
    }
}

async fn require_materializable_template(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    template_id: Uuid,
) -> Result<(), InstitutionError> {
    let valid: bool = sqlx::query_scalar(
        r"SELECT EXISTS(SELECT 1 FROM latex_core.templates t
           JOIN latex_core.template_files f ON f.template_id=t.id AND f.path=t.main_file
           WHERE t.id=$1 AND t.main_file IS NOT NULL)",
    )
    .bind(template_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(InstitutionError::Database)?;
    if valid {
        Ok(())
    } else {
        Err(InstitutionError::Conflict(
            "selected template must exist and contain its configured Main file".into(),
        ))
    }
}

fn page_from_rows<F>(
    rows: Vec<PgRow>,
    page: i64,
    limit: i64,
    offset: i64,
    mut decode: F,
) -> Result<PaperTeamPage, InstitutionError>
where
    F: FnMut(&PgRow) -> Result<Value, InstitutionError>,
{
    let total = rows
        .first()
        .map_or(Ok(0_i64), |row| row.try_get("total"))
        .map_err(InstitutionError::Database)?;
    let items = rows
        .iter()
        .map(&mut decode)
        .collect::<Result<Vec<_>, _>>()?;
    let returned = i64::try_from(items.len()).unwrap_or(i64::MAX);
    Ok(PaperTeamPage {
        total,
        has_more: offset.saturating_add(returned) < total,
        page,
        limit,
        items,
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
        let mut rows = parse_csv(
            "students.csv",
            None,
            bytes,
            ImportLimits::default(),
            ImportMode::AddOnly,
            0,
        )
        .expect("CSV parses");
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
                ImportLimits::default(),
                ImportMode::AddOnly,
                0,
            )
            .is_err()
        );
        let rows = parse_csv(
            "departments.csv",
            None,
            b"department_id\nnot-a-uuid\n",
            ImportLimits::default(),
            ImportMode::AddOnly,
            0,
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
        let rows = parse_xlsx(&valid, ImportLimits::default(), ImportMode::AddOnly, 0)
            .expect("valid XLSX parses");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].error.is_none());

        let formula = workbook_bytes(
            r#"<row r="1"><c r="A1" t="inlineStr"><is><t>reg_no</t></is></c><c r="B1" t="inlineStr"><is><t>name</t></is></c><c r="C1" t="inlineStr"><is><t>email</t></is></c><c r="D1" t="inlineStr"><is><t>programme_code</t></is></c></row><row r="2"><c r="A2"><f>CONCAT(&quot;S&quot;,&quot;1&quot;)</f><v>1</v></c><c r="B2" t="inlineStr"><is><t>Student</t></is></c><c r="C2" t="inlineStr"><is><t>student@example.edu</t></is></c><c r="D2" t="inlineStr"><is><t>CSE</t></is></c></row>"#,
        );
        let rows = parse_xlsx(&formula, ImportLimits::default(), ImportMode::AddOnly, 0)
            .expect("XLSX shape parses");
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
