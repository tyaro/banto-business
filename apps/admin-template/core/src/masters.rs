//! Phase 3 のマスタ: 作業分類 / 経費分類 / 内部原価レート
//! （docs/domain/schema.md §1）。conventions §2 に従い `tauri` / `axum` /
//! RBAC を知らない。
//!
//! 作業分類と経費分類はマイグレーションでシードする固定のコード表で、
//! アプリからは**読み取りのみ**。値の増減はマイグレーションで行う
//! （コードが採算計算・税計算の分岐に効くため、実行時に自由に増減させない）。
//!
//! 内部原価レートだけは運用中に変わるため更新できる。ただし
//! **採算計算はこのテーブルを参照しない**（CLAUDE.md 1.2）— 参照するのは
//! `work_logs.applied_rate` に焼き付けた値で、ここは新規入力時の既定値の
//! 供給源でしかない。

use banto_core::{BantoError, FieldError};
use banto_server::ServerEvent;
use banto_storage::{Db, Dialect};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// 作業分類（`work_categories`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkCategory {
    pub code: String,
    pub name: String,
    /// 実質時間単価（移動除く）の分母から外すか。移動の判定にコード文字列を
    /// 使わないためのフラグ（AGENTS.md 3.2）。
    #[sqlx(rename = "excluded_from_effective_rate")]
    pub excluded_from_effective_rate: i64,
    #[sqlx(rename = "sort_order")]
    pub sort_order: i64,
    pub active: i64,
    /// 現在の内部原価レート（円/時）。未設定なら `None`。
    #[sqlx(rename = "hourly_rate")]
    pub hourly_rate: Option<i64>,
}

/// 経費分類（`expense_categories`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseCategory {
    pub code: String,
    pub name: String,
    #[sqlx(rename = "default_tax_category")]
    pub default_tax_category: String,
    #[sqlx(rename = "sort_order")]
    pub sort_order: i64,
    pub active: i64,
}

/// 内部原価レートの設定ペイロード。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostRateInput {
    pub work_category_code: String,
    /// 時間単価（円）。0 も許す（社内調整など原価を載せない運用のため）。
    pub hourly_rate: i64,
}

/// 時間単価の上限（円）。i64 の範囲より遥かに手前で弾くのは桁の打ち間違いを
/// 入力時点で捕まえるため（`projects.rs` の金額上限と同じ考え方）。
const MAX_HOURLY_RATE: i64 = 1_000_000;

fn today_expr(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Sqlite => "date('now')",
        Dialect::Postgres => "CURRENT_DATE::text",
    }
}

/// マスタのサービス層（conventions §2）。
#[derive(Clone)]
pub struct MastersService {
    db: Db,
    events: Option<broadcast::Sender<ServerEvent>>,
}

impl MastersService {
    pub fn new(db: Db) -> Self {
        Self { db, events: None }
    }

    pub fn with_events(mut self, events: broadcast::Sender<ServerEvent>) -> Self {
        self.events = Some(events);
        self
    }

    fn notify_changed(&self) {
        if let Some(tx) = &self.events {
            let _ = tx.send(ServerEvent::ResourceChanged {
                resource: "cost-rates".to_string(),
            });
        }
    }

    /// 作業分類の一覧（現在のレート付き）。`ListParams` を取らないのは、
    /// 10 件程度の固定コード表であり、絞り込み・ページングの意味が無いため
    /// （`items` のような一覧リソースとは性質が違う）。
    pub async fn list_work_categories(&self) -> Result<Vec<WorkCategory>, BantoError> {
        const SQL: &str = "SELECT c.code, c.name, c.excluded_from_effective_rate, c.sort_order, \
             c.active, r.hourly_rate \
             FROM work_categories c LEFT JOIN cost_rates r ON r.work_category_code = c.code \
             ORDER BY c.sort_order";
        match &self.db {
            Db::Sqlite(pool) => sqlx::query_as::<_, WorkCategory>(SQL).fetch_all(pool).await,
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => sqlx::query_as::<_, WorkCategory>(SQL).fetch_all(pool).await,
        }
        .map_err(banto_storage::storage_error)
    }

    pub async fn list_expense_categories(&self) -> Result<Vec<ExpenseCategory>, BantoError> {
        const SQL: &str = "SELECT code, name, default_tax_category, sort_order, active \
             FROM expense_categories ORDER BY sort_order";
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, ExpenseCategory>(SQL)
                    .fetch_all(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, ExpenseCategory>(SQL)
                    .fetch_all(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }

    /// 作業分類のレートを設定する（upsert）。行が無い分類は「未設定」を
    /// 意味するので、0 円のダミー行を先に撒かない（`0011_cost_rates.sql`）。
    pub async fn set_cost_rate(&self, input: CostRateInput) -> Result<WorkCategory, BantoError> {
        let code = input.work_category_code.trim().to_string();
        let mut errors: Vec<FieldError> = Vec::new();
        if code.is_empty() {
            errors.push(FieldError {
                field: "workCategoryCode".to_string(),
                message: "必須項目です".to_string(),
            });
        }
        if input.hourly_rate < 0 {
            errors.push(FieldError {
                field: "hourlyRate".to_string(),
                message: "0以上で入力してください".to_string(),
            });
        } else if input.hourly_rate > MAX_HOURLY_RATE {
            errors.push(FieldError {
                field: "hourlyRate".to_string(),
                message: format!("{MAX_HOURLY_RATE}以下で入力してください"),
            });
        }
        if !errors.is_empty() {
            return Err(BantoError::Validation {
                field_errors: errors,
            });
        }

        // 存在しない分類コードを外部キー任せにすると素の DB エラーになり、
        // どの入力が悪いのか利用者に伝わらない（`projects.rs` と同じ方針）。
        let categories = self.list_work_categories().await?;
        if !categories.iter().any(|c| c.code == code) {
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: "workCategoryCode".to_string(),
                    message: "作業分類が見つかりません".to_string(),
                }],
            });
        }

        let dialect = self.db.dialect();
        let sql = format!(
            "INSERT INTO cost_rates (work_category_code, hourly_rate, updated_at) \
             VALUES ({}, {}, {}) ON CONFLICT (work_category_code) \
             DO UPDATE SET hourly_rate = excluded.hourly_rate, updated_at = {}",
            dialect.placeholder(1),
            dialect.placeholder(2),
            today_expr(dialect),
            today_expr(dialect),
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query(&sql)
                    .bind(&code)
                    .bind(input.hourly_rate)
                    .execute(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query(&sql)
                    .bind(&code)
                    .bind(input.hourly_rate)
                    .execute(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)?;
        self.notify_changed();

        let updated = self
            .list_work_categories()
            .await?
            .into_iter()
            .find(|c| c.code == code)
            .expect("category exists (checked above)");
        Ok(updated)
    }

    /// 新規 WorkLog 入力時に提示する既定単価。未設定なら `None` を返し、
    /// 呼び出し側（サービス／画面）が「単価未設定」として扱う。
    pub async fn default_rate_for(&self, code: &str) -> Result<Option<i64>, BantoError> {
        Ok(self
            .list_work_categories()
            .await?
            .into_iter()
            .find(|c| c.code == code)
            .and_then(|c| c.hourly_rate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate_memory;

    async fn service() -> MastersService {
        let pool = migrate_memory().await.expect("migrate_memory");
        MastersService::new(pool)
    }

    #[tokio::test]
    async fn work_categories_are_seeded_with_travel_excluded() {
        let svc = service().await;
        let categories = svc.list_work_categories().await.expect("list");
        assert_eq!(categories.len(), 10);
        // 並び順はマスタの sort_order に従う（表示順を画面側で持たない）。
        assert_eq!(categories[0].code, "DESIGN");

        let travel = categories
            .iter()
            .find(|c| c.code == "TRAVEL")
            .expect("TRAVEL exists");
        assert_eq!(travel.excluded_from_effective_rate, 1);
        // 「移動」だけが実質時間単価（移動除く）の分母から外れる。
        assert_eq!(
            categories
                .iter()
                .filter(|c| c.excluded_from_effective_rate == 1)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn expense_categories_are_seeded_with_standard_tax() {
        let svc = service().await;
        let categories = svc.list_expense_categories().await.expect("list");
        assert_eq!(categories.len(), 7);
        assert!(categories
            .iter()
            .all(|c| c.default_tax_category == "STANDARD_10"));
    }

    /// レート未設定は 0 円ではなく `None`。0 円のダミー行を撒くと
    /// 「単価0で記録した工数」と区別できなくなる。
    #[tokio::test]
    async fn rate_is_none_until_set_then_upserts() {
        let svc = service().await;
        assert_eq!(svc.default_rate_for("DESIGN").await.expect("rate"), None);

        let updated = svc
            .set_cost_rate(CostRateInput {
                work_category_code: "DESIGN".to_string(),
                hourly_rate: 6000,
            })
            .await
            .expect("set");
        assert_eq!(updated.hourly_rate, Some(6000));

        // 2回目は UPDATE（重複エラーにならない）。
        let updated = svc
            .set_cost_rate(CostRateInput {
                work_category_code: "DESIGN".to_string(),
                hourly_rate: 6500,
            })
            .await
            .expect("update");
        assert_eq!(updated.hourly_rate, Some(6500));
        assert_eq!(
            svc.default_rate_for("DESIGN").await.expect("rate"),
            Some(6500)
        );
    }

    #[tokio::test]
    async fn unknown_category_is_a_field_error() {
        let svc = service().await;
        let err = svc
            .set_cost_rate(CostRateInput {
                work_category_code: "NOPE".to_string(),
                hourly_rate: 1000,
            })
            .await
            .expect_err("unknown code");
        match err {
            BantoError::Validation { field_errors } => {
                assert_eq!(field_errors[0].field, "workCategoryCode")
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn negative_rate_is_rejected() {
        let svc = service().await;
        let err = svc
            .set_cost_rate(CostRateInput {
                work_category_code: "DESIGN".to_string(),
                hourly_rate: -1,
            })
            .await
            .expect_err("negative");
        match err {
            BantoError::Validation { field_errors } => {
                assert_eq!(field_errors[0].field, "hourlyRate")
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }
}
