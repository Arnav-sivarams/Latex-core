//! Narrow application-owned presentation settings.

use crate::{GlobalRole, V2Error, V2Repository};
use core_types::UserId;
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EditorPreferences {
    pub schema_version: u8,
    pub font_size_px: i16,
    pub theme: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BrandingSettings {
    pub schema_version: u8,
    pub logo_blob_hash: Option<String>,
    pub logo_media_type: Option<String>,
    pub logo_width: Option<i32>,
    pub logo_height: Option<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoveryStatus {
    pub schema_version: u8,
    pub last_successful_backup_at: Option<String>,
    pub last_backup_failure_at: Option<String>,
    pub last_backup_failure_phase: Option<String>,
    pub last_successful_restore_drill_at: Option<String>,
}

impl V2Repository {
    /// Returns secret-free operator backup and restore-drill timestamps.
    ///
    /// # Errors
    ///
    /// Returns an error when persistence fails.
    pub async fn recovery_status(&self) -> Result<RecoveryStatus, V2Error> {
        let row = sqlx::query(
            "SELECT \
             (SELECT occurred_at::text FROM latex_core.operator_recovery_events WHERE operation='BACKUP' AND status='SUCCESS' ORDER BY occurred_at DESC LIMIT 1) AS last_backup, \
             (SELECT occurred_at::text FROM latex_core.operator_recovery_events WHERE operation='BACKUP' AND status='FAILED' ORDER BY occurred_at DESC LIMIT 1) AS last_failure, \
             (SELECT phase FROM latex_core.operator_recovery_events WHERE operation='BACKUP' AND status='FAILED' ORDER BY occurred_at DESC LIMIT 1) AS last_failure_phase, \
             (SELECT occurred_at::text FROM latex_core.operator_recovery_events WHERE operation='RESTORE_DRILL' AND status='SUCCESS' ORDER BY occurred_at DESC LIMIT 1) AS last_restore",
        )
        .fetch_one(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        Ok(RecoveryStatus {
            schema_version: 1,
            last_successful_backup_at: row.try_get("last_backup").map_err(V2Error::Database)?,
            last_backup_failure_at: row.try_get("last_failure").map_err(V2Error::Database)?,
            last_backup_failure_phase: row
                .try_get("last_failure_phase")
                .map_err(V2Error::Database)?,
            last_successful_restore_drill_at: row
                .try_get("last_restore")
                .map_err(V2Error::Database)?,
        })
    }

    /// Returns the linked institutional name, falling back to account email.
    ///
    /// # Errors
    ///
    /// Returns an error when the account is absent or persistence fails.
    pub async fn display_name(&self, user: UserId) -> Result<String, V2Error> {
        sqlx::query_scalar(
            "SELECT COALESCE(NULLIF(trim(student.name),''),NULLIF(trim(faculty.name),''),NULLIF(trim(admin.name),''),credentials.email) \
             FROM latex_core.user_credentials credentials \
             LEFT JOIN vcap.student_user_links student_link ON student_link.user_id=credentials.user_id AND student_link.status='LINKED' \
             LEFT JOIN vcap.students student ON student.reg_no=student_link.reg_no \
             LEFT JOIN vcap.faculty_user_links faculty_link ON faculty_link.user_id=credentials.user_id AND faculty_link.status='LINKED' \
             LEFT JOIN vcap.faculty faculty ON faculty.faculty_id=faculty_link.faculty_id \
             LEFT JOIN vcap.admin_user_links admin_link ON admin_link.user_id=credentials.user_id AND admin_link.status='LINKED' \
             LEFT JOIN vcap.admins admin ON admin.admin_id=admin_link.admin_id \
             WHERE credentials.user_id=$1",
        )
        .bind(user.as_uuid())
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "account identity" })
    }

    /// Loads personal source-editor presentation preferences.
    ///
    /// # Errors
    ///
    /// Returns an error when the account is absent or persistence fails.
    pub async fn editor_preferences(&self, user: UserId) -> Result<EditorPreferences, V2Error> {
        let row = sqlx::query(
            "SELECT COALESCE(preference.font_size_px,14)::smallint AS font_size_px,COALESCE(preference.theme,'LIGHT') AS theme \
             FROM latex_core.users account LEFT JOIN latex_core.user_editor_preferences preference ON preference.user_id=account.id WHERE account.id=$1",
        )
        .bind(user.as_uuid())
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "account" })?;
        Ok(EditorPreferences {
            schema_version: 1,
            font_size_px: row.try_get("font_size_px").map_err(V2Error::Database)?,
            theme: row.try_get("theme").map_err(V2Error::Database)?,
        })
    }

    /// Persists personal source-editor presentation preferences.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid values, an absent account, or persistence failure.
    pub async fn set_editor_preferences(
        &self,
        user: UserId,
        font_size_px: i16,
        theme: &str,
    ) -> Result<EditorPreferences, V2Error> {
        if !(12..=26).contains(&font_size_px) || !matches!(theme, "LIGHT" | "DARK") {
            return Err(V2Error::Integrity {
                message: "editor font size must be 12–26 px and theme must be LIGHT or DARK".into(),
            });
        }
        sqlx::query(
            "INSERT INTO latex_core.user_editor_preferences (user_id,font_size_px,theme) VALUES ($1,$2,$3) \
             ON CONFLICT(user_id) DO UPDATE SET font_size_px=EXCLUDED.font_size_px,theme=EXCLUDED.theme,updated_at=statement_timestamp()",
        )
        .bind(user.as_uuid())
        .bind(font_size_px)
        .bind(theme)
        .execute(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        Ok(EditorPreferences {
            schema_version: 1,
            font_size_px,
            theme: theme.to_owned(),
        })
    }

    /// Loads the singleton application-branding record.
    ///
    /// # Errors
    ///
    /// Returns an error when persistence fails.
    pub async fn branding(&self) -> Result<BrandingSettings, V2Error> {
        let row = sqlx::query(
            "SELECT logo_blob_hash,logo_media_type,logo_width,logo_height FROM latex_core.application_branding WHERE singleton",
        )
        .fetch_one(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        Ok(BrandingSettings {
            schema_version: 1,
            logo_blob_hash: row.try_get("logo_blob_hash").map_err(V2Error::Database)?,
            logo_media_type: row.try_get("logo_media_type").map_err(V2Error::Database)?,
            logo_width: row.try_get("logo_width").map_err(V2Error::Database)?,
            logo_height: row.try_get("logo_height").map_err(V2Error::Database)?,
        })
    }

    /// Replaces application branding after an Admin-authorized upload.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is not an Admin or persistence fails.
    pub async fn set_branding(
        &self,
        admin: UserId,
        blob_hash: &str,
        media_type: &str,
        width: i32,
        height: i32,
    ) -> Result<BrandingSettings, V2Error> {
        require_admin(self, admin).await?;
        sqlx::query(
            "UPDATE latex_core.application_branding SET logo_blob_hash=$1,logo_media_type=$2,logo_width=$3,logo_height=$4,updated_by_user_id=$5,updated_at=statement_timestamp() WHERE singleton",
        )
        .bind(blob_hash).bind(media_type).bind(width).bind(height).bind(admin.as_uuid())
        .execute(self.database.pool()).await.map_err(V2Error::Database)?;
        self.branding().await
    }

    /// Resets application branding to the built-in wordmark.
    ///
    /// # Errors
    ///
    /// Returns an error when the actor is not an Admin or persistence fails.
    pub async fn clear_branding(&self, admin: UserId) -> Result<(), V2Error> {
        require_admin(self, admin).await?;
        sqlx::query(
            "UPDATE latex_core.application_branding SET logo_blob_hash=NULL,logo_media_type=NULL,logo_width=NULL,logo_height=NULL,updated_by_user_id=$1,updated_at=statement_timestamp() WHERE singleton",
        )
        .bind(admin.as_uuid()).execute(self.database.pool()).await.map_err(V2Error::Database)?;
        Ok(())
    }
}

async fn require_admin(repository: &V2Repository, user: UserId) -> Result<(), V2Error> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM latex_core.global_user_roles WHERE user_id=$1")
            .bind(user.as_uuid())
            .fetch_optional(repository.database.pool())
            .await
            .map_err(V2Error::Database)?;
    if role.as_deref() != Some(GlobalRole::Admin.as_str()) {
        return Err(V2Error::RoleForbidden {
            user_id: user,
            required: "Admin",
            actual: role
                .as_deref()
                .and_then(|value| value.parse().ok())
                .unwrap_or(GlobalRole::Writer),
        });
    }
    Ok(())
}
