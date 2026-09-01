//! V2.1 Team-Leader review submissions and Mentor annotation boundaries.

use crate::{
    GlobalRole, PaperKind, PaperStatus, V2Error, V2Repository, WriterPaper,
    governance::assert_content_policy,
};
use core_types::{UserId, WorkspaceId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewPaperSummary {
    pub schema_version: u8,
    pub id: Uuid,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub status: PaperStatus,
    pub current_version_id: Option<Uuid>,
    pub current_build_id: Option<Uuid>,
    pub current_state_hash: Option<String>,
    pub pdf_available: bool,
    pub open_review_count: u64,
    pub blocking_review_count: u64,
    pub latest_activity: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewRoundList {
    pub schema_version: u8,
    pub review_open: bool,
    pub current_review_round: Option<Value>,
    pub rounds: Vec<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewSourceAnchorInput {
    pub file_id: Uuid,
    pub encoded_relative_start: Option<Vec<u8>>,
    pub encoded_relative_end: Option<Vec<u8>>,
    pub quoted_text: String,
    pub context_hash: String,
    pub source_sequence: u64,
    pub source_version_id: Option<Uuid>,
    pub document_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewPdfAnchorInput {
    pub page: i32,
    pub normalized_rectangles: Value,
    pub mapping_status: String,
    pub mapped_file_id: Option<Uuid>,
    pub mapped_line: Option<i32>,
    pub mapped_column: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewThreadInput {
    pub thread_type: String,
    pub message: String,
    #[serde(default = "default_review_severity")]
    pub severity: String,
    #[serde(default = "default_review_category")]
    pub category: String,
    pub assigned_writer_user_id: Option<UserId>,
    pub due_at: Option<String>,
    pub source_anchor: Option<ReviewSourceAnchorInput>,
    pub pdf_anchor: Option<ReviewPdfAnchorInput>,
    pub suggested_replacement: Option<String>,
    pub section_label: Option<String>,
}

fn default_review_severity() -> String {
    "NOTE".to_owned()
}

fn default_review_category() -> String {
    "WRITING".to_owned()
}

impl V2Repository {
    pub async fn mentor_papers(&self, mentor: UserId) -> Result<Vec<ReviewPaperSummary>, V2Error> {
        require_global_role(self.database.pool(), mentor, GlobalRole::Mentor).await?;
        let rows = sqlx::query(
            "SELECT t.id,t.workspace_id,t.name,t.status,t.updated_at::text AS updated_at, \
                    cb.version_id AS current_version_id,s.current_build_id,cb.state_hash AS current_state_hash, \
                    EXISTS(SELECT 1 FROM latex_core.compilation_artifacts a WHERE a.job_id=cb.compile_job_id AND a.kind='pdf') AS pdf_available, \
                    count(rt.id) FILTER (WHERE rt.state IN ('OPEN','ADDRESSED','REOPENED')) AS open_review_count, \
                    count(rt.id) FILTER (WHERE rt.severity='BLOCKING' AND rt.state IN ('OPEN','ADDRESSED','REOPENED')) AS blocking_review_count, \
                    GREATEST(t.updated_at,COALESCE(max(rt.updated_at),t.updated_at),COALESCE(max(rr.opened_at),t.updated_at))::text AS latest_activity \
             FROM latex_core.paper_teams t \
             JOIN latex_core.paper_team_members member ON member.paper_team_id=t.id AND member.user_id=$1 \
             LEFT JOIN latex_core.v2_paper_build_state s ON s.workspace_id=t.workspace_id \
             LEFT JOIN latex_core.v2_paper_builds cb ON cb.id=s.current_build_id \
             LEFT JOIN latex_core.review_rounds rr ON rr.workspace_id=t.workspace_id \
             LEFT JOIN latex_core.review_threads rt ON rt.review_round_id=rr.id \
             GROUP BY t.id,t.workspace_id,t.name,t.status,t.updated_at,cb.version_id,s.current_build_id,cb.state_hash,cb.compile_job_id \
             ORDER BY GREATEST(t.updated_at,COALESCE(max(rt.updated_at),t.updated_at),COALESCE(max(rr.opened_at),t.updated_at)) DESC,t.id",
        )
        .bind(mentor.as_uuid())
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(decode_review_paper).collect()
    }

    pub async fn review_paper(
        &self,
        actor: UserId,
        paper_id: Uuid,
    ) -> Result<(WriterPaper, GlobalRole), V2Error> {
        let row = participant_row(self.database.pool(), actor, paper_id).await?;
        let role = GlobalRole::from_str(
            &row.try_get::<String, _>("role")
                .map_err(V2Error::Database)?,
        )?;
        let paper = WriterPaper {
            id: row.try_get("id").map_err(V2Error::Database)?,
            workspace_id: WorkspaceId::from_uuid(
                row.try_get("workspace_id").map_err(V2Error::Database)?,
            ),
            name: row.try_get("name").map_err(V2Error::Database)?,
            kind: PaperKind::Team,
            status: PaperStatus::from_str(
                &row.try_get::<String, _>("status")
                    .map_err(V2Error::Database)?,
            )?,
            is_team_leader: row.try_get("is_team_leader").map_err(V2Error::Database)?,
            updated_at: row.try_get("updated_at").map_err(V2Error::Database)?,
        };
        Ok((paper, role))
    }

    pub async fn review_team_writers(
        &self,
        actor: UserId,
        paper_id: Uuid,
    ) -> Result<Vec<Value>, V2Error> {
        self.review_paper(actor, paper_id).await?;
        let rows = sqlx::query(
            "SELECT m.user_id,c.email FROM latex_core.paper_team_members m \
             JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role='writer' \
             JOIN latex_core.user_credentials c ON c.user_id=m.user_id \
             WHERE m.paper_team_id=$1 ORDER BY c.email",
        )
        .bind(paper_id)
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter()
            .map(|row| {
                Ok(json!({
                    "user_id": row.try_get::<Uuid,_>("user_id").map_err(V2Error::Database)?,
                    "email": row.try_get::<String,_>("email").map_err(V2Error::Database)?,
                }))
            })
            .collect()
    }

    pub async fn review_rounds(
        &self,
        actor: UserId,
        paper_id: Uuid,
    ) -> Result<ReviewRoundList, V2Error> {
        let (paper, _) = self.review_paper(actor, paper_id).await?;
        let rows = sqlx::query(
            "SELECT rr.id,rr.round_number,rr.baseline_version_id,rr.baseline_build_id,rr.baseline_state_hash, \
                    rr.opened_by_mentor_user_id,rr.submitted_by_leader_writer_id,rr.status, \
                    rr.opened_at::text AS opened_at,rr.closed_at::text AS closed_at, \
                    count(rt.id) FILTER (WHERE rt.state IN ('OPEN','ADDRESSED','REOPENED')) AS open_threads, \
                    count(rt.id) FILTER (WHERE rt.severity='BLOCKING' AND rt.state IN ('OPEN','ADDRESSED','REOPENED')) AS blocking_threads \
             FROM latex_core.review_rounds rr LEFT JOIN latex_core.review_threads rt ON rt.review_round_id=rr.id \
             WHERE rr.workspace_id=$1 GROUP BY rr.id ORDER BY rr.round_number DESC",
        )
        .bind(paper.workspace_id.as_uuid())
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        let rounds = rows
            .into_iter()
            .map(round_json)
            .collect::<Result<Vec<_>, _>>()?;
        let current_review_round = rounds
            .iter()
            .find(|round| round["status"] == "OPEN_FOR_REVIEW")
            .cloned();
        Ok(ReviewRoundList {
            schema_version: 1,
            review_open: current_review_round.is_some(),
            current_review_round,
            rounds,
        })
    }

    pub async fn open_review_round(
        &self,
        leader: UserId,
        paper_id: Uuid,
        expected_state_hash: &str,
    ) -> Result<(Value, bool), V2Error> {
        let (paper, role) = self.review_paper(leader, paper_id).await?;
        if role != GlobalRole::Writer {
            return Err(forbidden(leader, role, "Paper Team Leader Writer"));
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        if lock_paper_team(&mut tx, paper_id).await? != PaperStatus::Active {
            return Err(V2Error::Conflict {
                entity: "active Paper Team review submission",
            });
        }
        require_team_leader(&mut tx, paper_id, leader).await?;
        let legacy_closed = sqlx::query(
            "UPDATE latex_core.review_rounds SET status='CLOSED',closed_at=statement_timestamp() \
             WHERE workspace_id=$1 AND status IN ('OPEN','OPEN_FOR_REVIEW') \
               AND NOT (status='OPEN_FOR_REVIEW' AND submitted_by_leader_writer_id IS NOT NULL \
                        AND baseline_build_id IS NOT NULL AND baseline_state_hash IS NOT NULL)",
        )
        .bind(paper.workspace_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .rows_affected();
        if legacy_closed > 0 {
            tracing::info!(%paper_id, %legacy_closed, "stale legacy review rounds retained as closed history");
        }
        if let Some(row) = current_review_round(&mut tx, paper.workspace_id).await? {
            let round = round_json(row)?;
            tx.commit().await.map_err(V2Error::Database)?;
            tracing::info!(%paper_id, round_id=%round["id"], "existing paper review returned idempotently");
            return Ok((round, false));
        }
        let baseline = sqlx::query(
            "SELECT b.version_id,b.id AS build_id,b.state_hash FROM latex_core.v2_paper_build_state s \
             JOIN latex_core.v2_paper_builds b ON b.id=s.current_build_id \
             WHERE s.workspace_id=$1 AND b.status='succeeded' AND b.state_hash=$2 \
               AND s.desired_state_hash=$2 \
               AND EXISTS (SELECT 1 FROM latex_core.compilation_artifacts a \
                           WHERE a.job_id=b.compile_job_id AND a.kind='pdf')",
        )
        .bind(paper.workspace_id.as_uuid())
        .bind(expected_state_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::Conflict {
            entity: "current review PDF",
        })?;
        let baseline_version_id: Uuid =
            baseline.try_get("version_id").map_err(V2Error::Database)?;
        let baseline_build_id: Uuid = baseline.try_get("build_id").map_err(V2Error::Database)?;
        let baseline_state_hash: String =
            baseline.try_get("state_hash").map_err(V2Error::Database)?;
        let number: i64 = sqlx::query_scalar(
            "SELECT COALESCE(max(round_number),0)+1 FROM latex_core.review_rounds WHERE workspace_id=$1",
        )
        .bind(paper.workspace_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        let id = Uuid::new_v4();
        let row = sqlx::query(
            "INSERT INTO latex_core.review_rounds \
             (id,paper_id,workspace_id,round_number,baseline_version_id,opened_by_mentor_user_id, \
              submitted_by_leader_writer_id,baseline_build_id,baseline_state_hash,status) \
             VALUES ($1,$2,$3,$4,$5,NULL,$6,$7,$8,'OPEN_FOR_REVIEW') \
             RETURNING id,round_number,baseline_version_id,baseline_build_id,baseline_state_hash, \
                       opened_by_mentor_user_id,submitted_by_leader_writer_id,status,opened_at::text AS opened_at, \
                       closed_at::text AS closed_at,0::bigint AS open_threads,0::bigint AS blocking_threads",
        )
        .bind(id)
        .bind(paper_id)
        .bind(paper.workspace_id.as_uuid())
        .bind(number)
        .bind(baseline_version_id)
        .bind(leader.as_uuid())
        .bind(baseline_build_id)
        .bind(&baseline_state_hash)
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        tracing::info!(%paper_id, round_id=%id, leader_writer_user_id=%leader, %baseline_version_id, %baseline_build_id, state_hash=%baseline_state_hash, "paper sent for review");
        Ok((round_json(row)?, true))
    }

    pub async fn close_review_round(
        &self,
        leader: UserId,
        paper_id: Uuid,
        round_id: Uuid,
    ) -> Result<Value, V2Error> {
        let (paper, role) = self.review_paper(leader, paper_id).await?;
        if role != GlobalRole::Writer {
            return Err(forbidden(leader, role, "Paper Team Leader Writer"));
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        let _status = lock_paper_team(&mut tx, paper_id).await?;
        require_team_leader(&mut tx, paper_id, leader).await?;
        let status: String = sqlx::query_scalar(
            "SELECT status FROM latex_core.review_rounds WHERE id=$1 AND paper_id=$2 AND workspace_id=$3 FOR UPDATE",
        )
        .bind(round_id)
        .bind(paper_id)
        .bind(paper.workspace_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound { entity: "review round" })?;
        if status != "OPEN_FOR_REVIEW" {
            return Err(V2Error::Conflict {
                entity: "review round transition",
            });
        }
        let row = sqlx::query(
            "UPDATE latex_core.review_rounds SET status='CLOSED',closed_at=statement_timestamp() WHERE id=$1 \
             RETURNING id,round_number,baseline_version_id,baseline_build_id,baseline_state_hash, \
                       opened_by_mentor_user_id,submitted_by_leader_writer_id,status,opened_at::text AS opened_at, \
                       closed_at::text AS closed_at,0::bigint AS open_threads,0::bigint AS blocking_threads",
        )
        .bind(round_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        tracing::info!(%paper_id, %round_id, leader_writer_user_id=%leader, "paper review closed");
        round_json(row)
    }

    pub async fn create_review_thread(
        &self,
        mentor: UserId,
        paper_id: Uuid,
        input: &ReviewThreadInput,
    ) -> Result<Uuid, V2Error> {
        let (paper, role) = self.review_paper(mentor, paper_id).await?;
        if role != GlobalRole::Mentor {
            return Err(forbidden(mentor, role, "assigned mentor"));
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        let _status = lock_paper_team(&mut tx, paper_id).await?;
        require_team_mentor(&mut tx, paper_id, mentor).await?;
        let round_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM latex_core.review_rounds WHERE workspace_id=$1 AND status='OPEN_FOR_REVIEW' ORDER BY round_number DESC LIMIT 1 FOR UPDATE",
        )
        .bind(paper.workspace_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::Conflict {
            entity: "paper review gate",
        })?;
        if let Some(writer) = input.assigned_writer_user_id {
            require_team_writer(&mut tx, paper_id, writer).await?;
        }
        if let Some(anchor) = &input.source_anchor {
            require_file(&mut tx, paper.workspace_id, anchor.file_id, false).await?;
        }
        let approval = if input.thread_type == "PAPER_APPROVAL" {
            Some(current_exact_build(&mut tx, paper.workspace_id).await?)
        } else {
            None
        };
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO latex_core.review_threads \
             (id,review_round_id,workspace_id,thread_type,severity,category,assigned_writer_user_id,due_at,created_by_mentor_user_id,section_label,approved_version_id,approved_build_id,approved_state_hash) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8::timestamptz,$9,$10,$11,$12,$13)",
        )
        .bind(id)
        .bind(round_id)
        .bind(paper.workspace_id.as_uuid())
        .bind(&input.thread_type)
        .bind(&input.severity)
        .bind(&input.category)
        .bind(input.assigned_writer_user_id.map(|value| *value.as_uuid()))
        .bind(&input.due_at)
        .bind(mentor.as_uuid())
        .bind(&input.section_label)
        .bind(approval.as_ref().map(|value| value.0))
        .bind(approval.as_ref().map(|value| value.1))
        .bind(approval.as_ref().map(|value| value.2.as_str()))
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        insert_message(&mut tx, id, mentor, &input.message).await?;
        if let Some(anchor) = &input.source_anchor {
            sqlx::query(
                "INSERT INTO latex_core.review_source_anchors \
                 (thread_id,file_id,encoded_relative_start,encoded_relative_end,quoted_text,context_hash,source_sequence,source_version_id,document_epoch) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
            )
            .bind(id)
            .bind(anchor.file_id)
            .bind(&anchor.encoded_relative_start)
            .bind(&anchor.encoded_relative_end)
            .bind(&anchor.quoted_text)
            .bind(&anchor.context_hash)
            .bind(to_i64(anchor.source_sequence, "source sequence")?)
            .bind(anchor.source_version_id)
            .bind(to_i64(anchor.document_epoch, "document epoch")?)
            .execute(&mut *tx)
            .await
            .map_err(V2Error::Database)?;
        }
        if let Some(anchor) = &input.pdf_anchor {
            insert_pdf_anchor(&mut tx, id, paper.workspace_id, anchor).await?;
        }
        let replacement = if input.thread_type == "SUGGESTION" {
            Some(
                input
                    .suggested_replacement
                    .as_deref()
                    .unwrap_or(input.message.as_str())
                    .trim(),
            )
        } else {
            None
        };
        if let Some(replacement) = replacement {
            sqlx::query(
                "INSERT INTO latex_core.review_suggestions (thread_id,replacement_text) VALUES ($1,$2)",
            )
            .bind(id)
            .bind(replacement)
            .execute(&mut *tx)
            .await
            .map_err(V2Error::Database)?;
        }
        tx.commit().await.map_err(V2Error::Database)?;
        tracing::info!(%paper_id, thread_id=%id, mentor_user_id=%mentor, thread_type=%input.thread_type, "review thread created");
        Ok(id)
    }

    pub async fn review_threads(
        &self,
        actor: UserId,
        paper_id: Uuid,
    ) -> Result<Vec<Value>, V2Error> {
        let (paper, _) = self.review_paper(actor, paper_id).await?;
        let rows = sqlx::query(
            "SELECT rt.id,rt.review_round_id,rr.round_number,rt.thread_type,rt.state,rt.severity,rt.category, \
                    rt.assigned_writer_user_id,assigned.email AS assigned_writer_email,rt.due_at::text AS due_at, \
                    rt.created_by_mentor_user_id,mentor.email AS mentor_email,rt.section_label,rt.approved_version_id,approved.workspace_version AS approved_workspace_version,rt.approved_build_id,rt.approved_state_hash, \
                    rt.created_at::text AS created_at,rt.updated_at::text AS updated_at,rt.resolved_at::text AS resolved_at, \
                    (SELECT jsonb_agg(jsonb_build_object('id',m.id,'author_user_id',m.author_user_id,'author_email',c.email,'body',m.body,'created_at',m.created_at) ORDER BY m.created_at,m.id) \
                     FROM latex_core.review_messages m JOIN latex_core.user_credentials c ON c.user_id=m.author_user_id WHERE m.thread_id=rt.id) AS messages, \
                    CASE WHEN sa.thread_id IS NULL THEN NULL ELSE jsonb_build_object('file_id',sa.file_id,'path',pf.path,'file_deleted',pf.tombstoned, \
                         'encoded_relative_start',CASE WHEN sa.encoded_relative_start IS NULL THEN NULL ELSE encode(sa.encoded_relative_start,'base64') END, \
                         'encoded_relative_end',CASE WHEN sa.encoded_relative_end IS NULL THEN NULL ELSE encode(sa.encoded_relative_end,'base64') END, \
                         'quoted_text',sa.quoted_text,'context_hash',sa.context_hash,'source_sequence',sa.source_sequence,'source_version_id',sa.source_version_id,'document_epoch',sa.document_epoch) END AS source_anchor, \
                    pdf.anchor AS pdf_anchor, \
                    CASE WHEN suggestion.thread_id IS NULL THEN NULL ELSE jsonb_build_object('replacement_text',suggestion.replacement_text,'status',suggestion.status, \
                         'accepted_by_writer_user_id',suggestion.accepted_by_writer_user_id,'responded_at',suggestion.responded_at,'rejection_reason',suggestion.rejection_reason) END AS suggestion \
             FROM latex_core.review_threads rt JOIN latex_core.review_rounds rr ON rr.id=rt.review_round_id \
             JOIN latex_core.user_credentials mentor ON mentor.user_id=rt.created_by_mentor_user_id \
             LEFT JOIN latex_core.user_credentials assigned ON assigned.user_id=rt.assigned_writer_user_id \
             LEFT JOIN latex_core.paper_versions approved ON approved.id=rt.approved_version_id \
             LEFT JOIN latex_core.review_source_anchors sa ON sa.thread_id=rt.id \
             LEFT JOIN latex_core.paper_files pf ON pf.file_id=sa.file_id \
             LEFT JOIN LATERAL (SELECT jsonb_build_object('id',pa.id,'artifact_id',pa.artifact_id,'build_id',pa.build_id,'page',pa.page, \
                    'normalized_rectangles',pa.normalized_rectangles,'mapping_status',CASE WHEN pf.tombstoned THEN 'SOURCE_DELETED' ELSE pa.mapping_status END, \
                    'mapped_file_id',pa.mapped_file_id,'mapped_line',pa.mapped_line,'mapped_column',pa.mapped_column,'created_at',pa.created_at) AS anchor \
                    FROM latex_core.review_pdf_anchors pa WHERE pa.thread_id=rt.id ORDER BY pa.created_at DESC LIMIT 1) pdf ON TRUE \
             LEFT JOIN latex_core.review_suggestions suggestion ON suggestion.thread_id=rt.id \
             WHERE rt.workspace_id=$1 ORDER BY rt.updated_at DESC,rt.id",
        )
        .bind(paper.workspace_id.as_uuid())
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(thread_json).collect()
    }

    pub async fn add_review_message(
        &self,
        actor: UserId,
        paper_id: Uuid,
        thread_id: Uuid,
        body: &str,
    ) -> Result<Uuid, V2Error> {
        let (paper, role) = self.review_paper(actor, paper_id).await?;
        if !matches!(role, GlobalRole::Writer | GlobalRole::Mentor) {
            return Err(forbidden(actor, role, "review participant"));
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_thread(&mut tx, paper.workspace_id, thread_id).await?;
        let id = insert_message(&mut tx, thread_id, actor, body).await?;
        sqlx::query(
            "UPDATE latex_core.review_threads SET updated_at=statement_timestamp() WHERE id=$1",
        )
        .bind(thread_id)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(id)
    }

    pub async fn transition_review_thread(
        &self,
        actor: UserId,
        paper_id: Uuid,
        thread_id: Uuid,
        target: &str,
    ) -> Result<(), V2Error> {
        let (paper, role) = self.review_paper(actor, paper_id).await?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        let (current, thread_type) =
            lock_thread_state_and_type(&mut tx, paper.workspace_id, thread_id).await?;
        let valid = match role {
            GlobalRole::Writer => match target {
                "ADDRESSED" => matches!(current.as_str(), "OPEN" | "REOPENED"),
                "RESOLVED" => {
                    matches!(
                        thread_type.as_str(),
                        "COMMENT" | "SUGGESTION" | "SUGGESTED_REPLACEMENT"
                    ) && matches!(current.as_str(), "OPEN" | "ADDRESSED" | "REOPENED")
                }
                _ => false,
            },
            GlobalRole::Mentor => {
                (target == "RESOLVED" && current == "ADDRESSED")
                    || (target == "REOPENED"
                        && matches!(current.as_str(), "ADDRESSED" | "RESOLVED"))
            }
            GlobalRole::Admin => false,
        };
        if !valid {
            return Err(V2Error::Conflict {
                entity: "review thread transition",
            });
        }
        sqlx::query(
            "UPDATE latex_core.review_threads SET state=$2,updated_at=statement_timestamp(), \
             resolved_at=CASE WHEN $2='RESOLVED' THEN statement_timestamp() ELSE NULL END WHERE id=$1",
        )
        .bind(thread_id)
        .bind(target)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        tracing::info!(%paper_id, %thread_id, actor_user_id=%actor, %target, "review thread transitioned");
        Ok(())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the Mentor-controlled fields are explicit at the transaction boundary"
    )]
    pub async fn update_review_controls(
        &self,
        mentor: UserId,
        paper_id: Uuid,
        thread_id: Uuid,
        severity: &str,
        category: &str,
        assigned_writer: Option<UserId>,
        due_at: Option<&str>,
    ) -> Result<(), V2Error> {
        let (paper, role) = self.review_paper(mentor, paper_id).await?;
        if role != GlobalRole::Mentor {
            return Err(forbidden(mentor, role, "assigned mentor"));
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_thread(&mut tx, paper.workspace_id, thread_id).await?;
        if let Some(writer) = assigned_writer {
            require_team_writer(&mut tx, paper_id, writer).await?;
        }
        sqlx::query(
            "UPDATE latex_core.review_threads SET severity=$2,category=$3,assigned_writer_user_id=$4,due_at=$5::timestamptz,updated_at=statement_timestamp() WHERE id=$1",
        )
        .bind(thread_id)
        .bind(severity)
        .bind(category)
        .bind(assigned_writer.map(|value| *value.as_uuid()))
        .bind(due_at)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)
    }

    pub async fn accept_review_suggestion(
        &self,
        writer: UserId,
        paper_id: Uuid,
        thread_id: Uuid,
        durable_sequence: u64,
    ) -> Result<(), V2Error> {
        let (paper, role) = self.review_paper(writer, paper_id).await?;
        if role != GlobalRole::Writer {
            return Err(forbidden(writer, role, "assigned writer"));
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_thread(&mut tx, paper.workspace_id, thread_id).await?;
        let file_id: Uuid = sqlx::query_scalar(
            "SELECT file_id FROM latex_core.review_source_anchors WHERE thread_id=$1",
        )
        .bind(thread_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::Conflict {
            entity: "resolvable suggestion source anchor",
        })?;
        assert_content_policy(&mut tx, file_id).await?;
        let writer_edit: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM latex_core.collaboration_updates u \
             JOIN latex_core.review_threads rt ON rt.id=$1 \
             WHERE u.id=$2 AND u.workspace_id=rt.workspace_id AND u.file_id=$3 \
               AND u.actor_user_id=$4 AND u.created_at>=rt.created_at)",
        )
        .bind(thread_id)
        .bind(to_i64(durable_sequence, "durable sequence")?)
        .bind(file_id)
        .bind(writer.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        if !writer_edit {
            return Err(V2Error::Conflict {
                entity: "Writer durable suggestion edit",
            });
        }
        let changed = sqlx::query(
            "UPDATE latex_core.review_suggestions SET status='ACCEPTED',accepted_by_writer_user_id=$2,responded_at=statement_timestamp() \
             WHERE thread_id=$1 AND status='PENDING'",
        )
        .bind(thread_id)
        .bind(writer.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        if changed.rows_affected() != 1 {
            return Err(V2Error::Conflict {
                entity: "pending suggestion",
            });
        }
        sqlx::query(
            "UPDATE latex_core.review_threads SET state='ADDRESSED',updated_at=statement_timestamp() WHERE id=$1",
        )
        .bind(thread_id)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        tracing::info!(%paper_id, %thread_id, writer_user_id=%writer, durable_sequence, "Writer accepted suggestion after durable Yjs edit");
        Ok(())
    }

    pub async fn reject_review_suggestion(
        &self,
        writer: UserId,
        paper_id: Uuid,
        thread_id: Uuid,
        reason: Option<&str>,
    ) -> Result<(), V2Error> {
        let (paper, role) = self.review_paper(writer, paper_id).await?;
        if role != GlobalRole::Writer {
            return Err(forbidden(writer, role, "assigned writer"));
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_thread(&mut tx, paper.workspace_id, thread_id).await?;
        let changed = sqlx::query(
            "UPDATE latex_core.review_suggestions SET status='REJECTED',responded_at=statement_timestamp(),rejection_reason=$2 \
             WHERE thread_id=$1 AND status='PENDING'",
        )
        .bind(thread_id)
        .bind(reason)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        if changed.rows_affected() != 1 {
            return Err(V2Error::Conflict {
                entity: "pending suggestion",
            });
        }
        sqlx::query(
            "UPDATE latex_core.review_threads SET state='REJECTED',updated_at=statement_timestamp() WHERE id=$1",
        )
        .bind(thread_id)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)
    }

    pub async fn review_activity(
        &self,
        actor: UserId,
        paper_id: Uuid,
    ) -> Result<Vec<Value>, V2Error> {
        let (paper, _) = self.review_paper(actor, paper_id).await?;
        let rows = sqlx::query(
            "SELECT event_type,summary,occurred_at::text AS occurred_at FROM ( \
               SELECT 'review_thread' AS event_type,thread_type||' created' AS summary,created_at AS occurred_at FROM latex_core.review_threads WHERE workspace_id=$1 \
               UNION ALL SELECT 'review_message',c.email||' replied',m.created_at FROM latex_core.review_messages m JOIN latex_core.review_threads rt ON rt.id=m.thread_id JOIN latex_core.user_credentials c ON c.user_id=m.author_user_id WHERE rt.workspace_id=$1 \
               UNION ALL SELECT 'review_state',thread_type||' '||lower(state),updated_at FROM latex_core.review_threads WHERE workspace_id=$1 AND updated_at>created_at \
               UNION ALL SELECT 'suggestion','Suggestion '||lower(s.status),s.responded_at FROM latex_core.review_suggestions s JOIN latex_core.review_threads rt ON rt.id=s.thread_id WHERE rt.workspace_id=$1 AND s.responded_at IS NOT NULL \
               UNION ALL SELECT 'review_round','Review round '||round_number||' '||lower(status),COALESCE(closed_at,opened_at) FROM latex_core.review_rounds WHERE workspace_id=$1 \
               UNION ALL SELECT 'version',version_type||' version '||version_number,created_at FROM latex_core.paper_versions WHERE workspace_id=$1 \
               UNION ALL SELECT 'compile','Compile '||status,updated_at FROM latex_core.v2_paper_builds WHERE workspace_id=$1 \
             ) events ORDER BY occurred_at DESC LIMIT 200",
        )
        .bind(paper.workspace_id.as_uuid())
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter()
            .map(|row| {
                Ok(json!({
                    "event_type":row.try_get::<String,_>("event_type").map_err(V2Error::Database)?,
                    "summary":row.try_get::<String,_>("summary").map_err(V2Error::Database)?,
                    "occurred_at":row.try_get::<String,_>("occurred_at").map_err(V2Error::Database)?,
                }))
            })
            .collect()
    }
}

async fn participant_row(
    pool: &sqlx::PgPool,
    actor: UserId,
    paper_id: Uuid,
) -> Result<PgRow, V2Error> {
    sqlx::query(
        "SELECT t.id,t.workspace_id,t.name,t.status,t.updated_at::text AS updated_at,r.role,m.is_leader AS is_team_leader \
         FROM latex_core.paper_teams t JOIN latex_core.paper_team_members m ON m.paper_team_id=t.id AND m.user_id=$2 \
         JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role IN ('writer','mentor') WHERE t.id=$1",
    )
    .bind(paper_id)
    .bind(actor.as_uuid())
    .fetch_optional(pool)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound { entity: "review paper" })
}

async fn require_global_role(
    pool: &sqlx::PgPool,
    actor: UserId,
    expected: GlobalRole,
) -> Result<(), V2Error> {
    let role: String =
        sqlx::query_scalar("SELECT role FROM latex_core.global_user_roles WHERE user_id=$1")
            .bind(actor.as_uuid())
            .fetch_optional(pool)
            .await
            .map_err(V2Error::Database)?
            .ok_or(V2Error::RoleMissing { user_id: actor })?;
    let actual = GlobalRole::from_str(&role)?;
    if actual != expected {
        return Err(forbidden(actor, actual, "required review role"));
    }
    Ok(())
}

const fn forbidden(actor: UserId, actual: GlobalRole, required: &'static str) -> V2Error {
    V2Error::RoleForbidden {
        user_id: actor,
        required,
        actual,
    }
}

async fn require_team_writer(
    tx: &mut Transaction<'_, Postgres>,
    paper_id: Uuid,
    writer: UserId,
) -> Result<(), V2Error> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM latex_core.paper_team_members m JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role='writer' WHERE m.paper_team_id=$1 AND m.user_id=$2)",
    ).bind(paper_id).bind(writer.as_uuid()).fetch_one(&mut **tx).await.map_err(V2Error::Database)?;
    if !exists {
        return Err(V2Error::NotFound {
            entity: "assigned Writer",
        });
    }
    Ok(())
}

async fn require_team_leader(
    tx: &mut Transaction<'_, Postgres>,
    paper_id: Uuid,
    writer: UserId,
) -> Result<(), V2Error> {
    let is_leader = sqlx::query_scalar::<_, Uuid>(
        "SELECT m.user_id FROM latex_core.paper_team_members m \
         JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role='writer' \
         WHERE m.paper_team_id=$1 AND m.user_id=$2 AND m.is_leader FOR UPDATE OF m",
    )
    .bind(paper_id)
    .bind(writer.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .is_some();
    if !is_leader {
        return Err(V2Error::RoleForbidden {
            user_id: writer,
            required: "Paper Team Leader Writer",
            actual: GlobalRole::Writer,
        });
    }
    Ok(())
}

async fn require_team_mentor(
    tx: &mut Transaction<'_, Postgres>,
    paper_id: Uuid,
    mentor: UserId,
) -> Result<(), V2Error> {
    let is_assigned = sqlx::query_scalar::<_, Uuid>(
        "SELECT m.user_id FROM latex_core.paper_team_members m \
         JOIN latex_core.global_user_roles r ON r.user_id=m.user_id AND r.role='mentor' \
         WHERE m.paper_team_id=$1 AND m.user_id=$2 FOR UPDATE OF m",
    )
    .bind(paper_id)
    .bind(mentor.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .is_some();
    if !is_assigned {
        return Err(V2Error::RoleForbidden {
            user_id: mentor,
            required: "assigned Paper Team Mentor",
            actual: GlobalRole::Mentor,
        });
    }
    Ok(())
}

async fn lock_paper_team(
    tx: &mut Transaction<'_, Postgres>,
    paper_id: Uuid,
) -> Result<PaperStatus, V2Error> {
    let status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM latex_core.paper_teams WHERE id=$1 FOR UPDATE",
    )
    .bind(paper_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound {
        entity: "Paper Team",
    })?;
    PaperStatus::from_str(&status)
}

async fn require_file(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
    file_id: Uuid,
    allow_deleted: bool,
) -> Result<(), V2Error> {
    let found: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM latex_core.paper_files WHERE workspace_id=$1 AND file_id=$2 AND ($3 OR NOT tombstoned))",
    ).bind(workspace_id.as_uuid()).bind(file_id).bind(allow_deleted).fetch_one(&mut **tx).await.map_err(V2Error::Database)?;
    if !found {
        return Err(V2Error::NotFound {
            entity: "review source file",
        });
    }
    Ok(())
}

async fn lock_thread(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
    thread_id: Uuid,
) -> Result<String, V2Error> {
    sqlx::query_scalar(
        "SELECT state FROM latex_core.review_threads WHERE id=$1 AND workspace_id=$2 FOR UPDATE",
    )
    .bind(thread_id)
    .bind(workspace_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound {
        entity: "review thread",
    })
}

async fn current_review_round(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
) -> Result<Option<PgRow>, V2Error> {
    sqlx::query(
        "SELECT rr.id,rr.round_number,rr.baseline_version_id,rr.baseline_build_id,rr.baseline_state_hash, \
                rr.opened_by_mentor_user_id,rr.submitted_by_leader_writer_id,rr.status, \
                rr.opened_at::text AS opened_at,rr.closed_at::text AS closed_at, \
                (SELECT count(*) FROM latex_core.review_threads rt WHERE rt.review_round_id=rr.id \
                    AND rt.state IN ('OPEN','ADDRESSED','REOPENED')) AS open_threads, \
                (SELECT count(*) FROM latex_core.review_threads rt WHERE rt.review_round_id=rr.id \
                    AND rt.severity='BLOCKING' AND rt.state IN ('OPEN','ADDRESSED','REOPENED')) AS blocking_threads \
         FROM latex_core.review_rounds rr \
         WHERE rr.workspace_id=$1 AND rr.status='OPEN_FOR_REVIEW' \
         ORDER BY rr.round_number DESC LIMIT 1 FOR UPDATE",
    )
    .bind(workspace_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)
}

async fn lock_thread_state_and_type(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
    thread_id: Uuid,
) -> Result<(String, String), V2Error> {
    let row = sqlx::query(
        "SELECT state,thread_type FROM latex_core.review_threads \
         WHERE id=$1 AND workspace_id=$2 FOR UPDATE",
    )
    .bind(thread_id)
    .bind(workspace_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound {
        entity: "review thread",
    })?;
    Ok((
        row.try_get("state").map_err(V2Error::Database)?,
        row.try_get("thread_type").map_err(V2Error::Database)?,
    ))
}

async fn insert_message(
    tx: &mut Transaction<'_, Postgres>,
    thread_id: Uuid,
    author: UserId,
    body: &str,
) -> Result<Uuid, V2Error> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO latex_core.review_messages (id,thread_id,author_user_id,body) VALUES ($1,$2,$3,$4)")
        .bind(id).bind(thread_id).bind(author.as_uuid()).bind(body.trim()).execute(&mut **tx).await.map_err(V2Error::Database)?;
    Ok(id)
}

async fn current_exact_build(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
) -> Result<(Uuid, Uuid, String), V2Error> {
    let row = sqlx::query(
        "SELECT b.version_id,b.id,b.state_hash FROM latex_core.v2_paper_build_state s JOIN latex_core.v2_paper_builds b ON b.id=s.current_build_id WHERE s.workspace_id=$1 AND b.status='succeeded'",
    ).bind(workspace_id.as_uuid()).fetch_optional(&mut **tx).await.map_err(V2Error::Database)?
        .ok_or(V2Error::Conflict { entity: "current exact build" })?;
    Ok((
        row.try_get("version_id").map_err(V2Error::Database)?,
        row.try_get("id").map_err(V2Error::Database)?,
        row.try_get("state_hash").map_err(V2Error::Database)?,
    ))
}

async fn insert_pdf_anchor(
    tx: &mut Transaction<'_, Postgres>,
    thread_id: Uuid,
    workspace_id: WorkspaceId,
    anchor: &ReviewPdfAnchorInput,
) -> Result<(), V2Error> {
    let row = sqlx::query(
        "SELECT b.id,a.artifact_id FROM latex_core.v2_paper_build_state s JOIN latex_core.v2_paper_builds b ON b.id=s.current_build_id \
         JOIN latex_core.compilation_artifacts a ON a.job_id=b.compile_job_id AND a.kind='pdf' WHERE s.workspace_id=$1 ORDER BY a.logical_name LIMIT 1",
    ).bind(workspace_id.as_uuid()).fetch_optional(&mut **tx).await.map_err(V2Error::Database)?
        .ok_or(V2Error::Conflict { entity: "current PDF artifact" })?;
    sqlx::query(
        "INSERT INTO latex_core.review_pdf_anchors (id,thread_id,artifact_id,build_id,page,normalized_rectangles,mapping_status,mapped_file_id,mapped_line,mapped_column) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    ).bind(Uuid::new_v4()).bind(thread_id).bind(row.try_get::<Uuid,_>("artifact_id").map_err(V2Error::Database)?)
      .bind(row.try_get::<Uuid,_>("id").map_err(V2Error::Database)?).bind(anchor.page).bind(&anchor.normalized_rectangles)
      .bind(&anchor.mapping_status).bind(anchor.mapped_file_id).bind(anchor.mapped_line).bind(anchor.mapped_column)
      .execute(&mut **tx).await.map_err(V2Error::Database)?;
    Ok(())
}

fn decode_review_paper(row: PgRow) -> Result<ReviewPaperSummary, V2Error> {
    let open: i64 = row
        .try_get("open_review_count")
        .map_err(V2Error::Database)?;
    let blocking: i64 = row
        .try_get("blocking_review_count")
        .map_err(V2Error::Database)?;
    Ok(ReviewPaperSummary {
        schema_version: 1,
        id: row.try_get("id").map_err(V2Error::Database)?,
        workspace_id: WorkspaceId::from_uuid(
            row.try_get("workspace_id").map_err(V2Error::Database)?,
        ),
        name: row.try_get("name").map_err(V2Error::Database)?,
        status: PaperStatus::from_str(
            &row.try_get::<String, _>("status")
                .map_err(V2Error::Database)?,
        )?,
        current_version_id: row
            .try_get("current_version_id")
            .map_err(V2Error::Database)?,
        current_build_id: row.try_get("current_build_id").map_err(V2Error::Database)?,
        current_state_hash: row
            .try_get("current_state_hash")
            .map_err(V2Error::Database)?,
        pdf_available: row.try_get("pdf_available").map_err(V2Error::Database)?,
        open_review_count: u64::try_from(open).map_err(|_| V2Error::Integrity {
            message: "negative review count".into(),
        })?,
        blocking_review_count: u64::try_from(blocking).map_err(|_| V2Error::Integrity {
            message: "negative blocking count".into(),
        })?,
        latest_activity: row.try_get("latest_activity").map_err(V2Error::Database)?,
    })
}

fn round_json(row: PgRow) -> Result<Value, V2Error> {
    Ok(
        json!({"schema_version":1,"id":row.try_get::<Uuid,_>("id").map_err(V2Error::Database)?,"round_number":row.try_get::<i64,_>("round_number").map_err(V2Error::Database)?,"baseline_version_id":row.try_get::<Uuid,_>("baseline_version_id").map_err(V2Error::Database)?,"baseline_build_id":row.try_get::<Option<Uuid>,_>("baseline_build_id").map_err(V2Error::Database)?,"baseline_state_hash":row.try_get::<Option<String>,_>("baseline_state_hash").map_err(V2Error::Database)?,"opened_by_mentor_user_id":row.try_get::<Option<Uuid>,_>("opened_by_mentor_user_id").map_err(V2Error::Database)?,"submitted_by_leader_writer_id":row.try_get::<Option<Uuid>,_>("submitted_by_leader_writer_id").map_err(V2Error::Database)?,"status":row.try_get::<String,_>("status").map_err(V2Error::Database)?,"opened_at":row.try_get::<String,_>("opened_at").map_err(V2Error::Database)?,"closed_at":row.try_get::<Option<String>,_>("closed_at").map_err(V2Error::Database)?,"open_threads":row.try_get::<i64,_>("open_threads").map_err(V2Error::Database)?,"blocking_threads":row.try_get::<i64,_>("blocking_threads").map_err(V2Error::Database)?}),
    )
}

fn thread_json(row: PgRow) -> Result<Value, V2Error> {
    Ok(json!({
        "schema_version": 1,
        "id": row.try_get::<Uuid,_>("id").map_err(V2Error::Database)?,
        "review_round_id": row.try_get::<Uuid,_>("review_round_id").map_err(V2Error::Database)?,
        "round_number": row.try_get::<i64,_>("round_number").map_err(V2Error::Database)?,
        "thread_type": row.try_get::<String,_>("thread_type").map_err(V2Error::Database)?,
        "state": row.try_get::<String,_>("state").map_err(V2Error::Database)?,
        "severity": row.try_get::<String,_>("severity").map_err(V2Error::Database)?,
        "category": row.try_get::<String,_>("category").map_err(V2Error::Database)?,
        "assigned_writer_user_id": row.try_get::<Option<Uuid>,_>("assigned_writer_user_id").map_err(V2Error::Database)?,
        "assigned_writer_email": row.try_get::<Option<String>,_>("assigned_writer_email").map_err(V2Error::Database)?,
        "due_at": row.try_get::<Option<String>,_>("due_at").map_err(V2Error::Database)?,
        "created_by_mentor_user_id": row.try_get::<Uuid,_>("created_by_mentor_user_id").map_err(V2Error::Database)?,
        "mentor_email": row.try_get::<String,_>("mentor_email").map_err(V2Error::Database)?,
        "section_label": row.try_get::<Option<String>,_>("section_label").map_err(V2Error::Database)?,
        "approved_version_id": row.try_get::<Option<Uuid>,_>("approved_version_id").map_err(V2Error::Database)?,
        "approved_workspace_version": row.try_get::<Option<i64>,_>("approved_workspace_version").map_err(V2Error::Database)?,
        "approved_build_id": row.try_get::<Option<Uuid>,_>("approved_build_id").map_err(V2Error::Database)?,
        "approved_state_hash": row.try_get::<Option<String>,_>("approved_state_hash").map_err(V2Error::Database)?,
        "created_at": row.try_get::<String,_>("created_at").map_err(V2Error::Database)?,
        "updated_at": row.try_get::<String,_>("updated_at").map_err(V2Error::Database)?,
        "resolved_at": row.try_get::<Option<String>,_>("resolved_at").map_err(V2Error::Database)?,
        "messages": row.try_get::<Option<Value>,_>("messages").map_err(V2Error::Database)?.unwrap_or_else(||json!([])),
        "source_anchor": row.try_get::<Option<Value>,_>("source_anchor").map_err(V2Error::Database)?,
        "pdf_anchor": row.try_get::<Option<Value>,_>("pdf_anchor").map_err(V2Error::Database)?,
        "suggestion": row.try_get::<Option<Value>,_>("suggestion").map_err(V2Error::Database)?,
    }))
}

fn to_i64(value: u64, field: &str) -> Result<i64, V2Error> {
    i64::try_from(value).map_err(|_| V2Error::Integrity {
        message: format!("{field} exceeds PostgreSQL BIGINT"),
    })
}
