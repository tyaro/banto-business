//! App-layer PostgreSQL smoke test (V2 "PostgreSQL アプリ全体対応", PR3).
//!
//! This is the first place the whole app service stack is exercised against a
//! real PostgreSQL server: `init_db_from_target` runs `migrations-postgres` +
//! the demo seed, then the four core services (`items`/`users`/`settings`/
//! `audit`) perform representative CRUD. It proves the strict-typed Postgres
//! DDL and the service layer's Postgres enum-dispatch arms actually work
//! end-to-end - not just that they compile (which PR2 already guaranteed).
//!
//! Gated two ways so it never breaks a PostgreSQL-less build:
//! - The whole file is `#![cfg(feature = "postgres")]`, so a default
//!   (SQLite-only) build does not even compile it.
//! - At runtime it early-returns unless `BANTO_TEST_PG_URL` is set, so a local
//!   `cargo test --features postgres` with no server still passes. CI's
//!   `app-postgres` job sets the env var against a `postgres:16` service
//!   container (mirroring the existing `storage-postgres` job).
#![cfg(feature = "postgres")]

use admin_template_core::audit::{AuditEntry, AuditLogService};
use admin_template_core::backup::BackupService;
use admin_template_core::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
use admin_template_core::db::init_db_from_target;
use admin_template_core::expenses::{ExpenseInput, ExpensesService};
use admin_template_core::invoices::{InvoiceInput, InvoiceLineInput, InvoicesService};
use admin_template_core::issuer::{IssuerInput, IssuerService};
use admin_template_core::items::{ImportResult, ItemImportRow, ItemInput, ItemsService};
use admin_template_core::masters::{CostRateInput, MastersService};
use admin_template_core::profitability::ProfitabilityService;
use admin_template_core::projects::{ProjectInput, ProjectsService};
use admin_template_core::settings::SettingsService;
use admin_template_core::users::{Role, UsersService};
use admin_template_core::work_logs::{WorkLogInput, WorkLogsService};
use banto_core::ListParams;
use std::path::PathBuf;

/// Drop every table this app owns (plus sqlx's migration bookkeeping) so the
/// smoke test starts from a clean schema even if a previous run left state
/// behind. `CASCADE` also drops the identity sequences. Uses the public
/// `connect_postgres` helper to get a raw pool for the DDL.
async fn reset_schema(url: &str) {
    let pool = banto_storage::connect_postgres(url)
        .await
        .expect("connect for schema reset");
    for stmt in [
        "DROP TABLE IF EXISTS attachments, audit_log, users, settings, items CASCADE",
        // Business ドメイン（Phase 2〜）。テンプレート由来のテーブルだけを
        // 落としていると、2回目以降のローカル実行でマイグレーションが
        // 「既に存在する」で失敗する（CI は毎回新しいコンテナなので出ない）。
        "DROP TABLE IF EXISTS invoice_tax_summaries, invoice_lines, invoices, expenses, \
         work_logs, trips, cost_rates, work_categories, expense_categories, projects, \
         customers CASCADE",
        "DROP TABLE IF EXISTS _sqlx_migrations",
    ] {
        sqlx::query(stmt)
            .execute(&pool)
            .await
            .expect("drop for reset");
    }
    pool.close().await;
}

#[tokio::test]
async fn app_layer_crud_round_trips_on_postgres() {
    let Ok(url) = std::env::var("BANTO_TEST_PG_URL") else {
        eprintln!("pg_smoke: BANTO_TEST_PG_URL unset - skipping (no PostgreSQL server)");
        return;
    };

    reset_schema(&url).await;

    // migrations-postgres + deterministic 1,000-row seed.
    let db = init_db_from_target(&url)
        .await
        .expect("init_db_from_target should run migrations-postgres and seed");

    // --- items: seed count, create, update, list, delete ---------------------
    let items = ItemsService::new(db.clone());

    let seeded = items
        .list(ListParams::default())
        .await
        .expect("items list after seed");
    assert_eq!(
        seeded.total_count, 1_000,
        "seed should insert exactly 1,000 rows"
    );

    // A create must land AFTER the seeded ids (the identity sequence was
    // advanced past the explicit 1..=1000 seed), proving id order == insertion
    // order still holds.
    let created = items
        .create(ItemInput {
            name: "テスト商品".to_string(),
            price: 123,
            stock: 7,
        })
        .await
        .expect("items create");
    assert!(
        created.id > 1_000,
        "new id must be past the seed, got {}",
        created.id
    );
    assert_eq!(created.price, 123);
    assert_eq!(
        created.updated_at.len(),
        10,
        "updated_at is a YYYY-MM-DD date"
    );

    let updated = items
        .update(
            created.id,
            ItemInput {
                name: "テスト商品(改)".to_string(),
                price: 200,
                stock: 3,
            },
        )
        .await
        .expect("items update");
    assert_eq!(updated.price, 200);
    assert_eq!(updated.stock, 3);

    items.delete(created.id).await.expect("items delete");
    let after_delete = items
        .list(ListParams::default())
        .await
        .expect("items list after delete");
    assert_eq!(after_delete.total_count, 1_000);

    // --- items import: round-trip + all-or-nothing rollback (spec M15) --------
    // Exercises `import_apply_postgres` - the hand-written Postgres mirror of
    // the SQLite transaction body that no other test or CI path reached
    // (M-review 2026-08 M-12). Both of its branches (commit / rollback) run
    // here against real Postgres.
    //
    // Round-trip: two INSERTs (`id: None`) plus one UPDATE of a seeded row
    // (ids 1..=1000) commit together.
    let import_ok = items
        .import(vec![
            ItemImportRow {
                id: None,
                name: "取込A".to_string(),
                price: 111,
                stock: 1,
            },
            ItemImportRow {
                id: None,
                name: "取込B".to_string(),
                price: 222,
                stock: 2,
            },
            ItemImportRow {
                id: Some(1),
                name: "既存1(改)".to_string(),
                price: 333,
                stock: 3,
            },
        ])
        .await
        .expect("items import (round-trip)");
    assert_eq!(
        import_ok,
        ImportResult {
            created: 2,
            updated: 1,
            errors: Vec::new(),
        }
    );
    let after_import = items
        .list(ListParams::default())
        .await
        .expect("items list after import");
    assert_eq!(
        after_import.total_count, 1_002,
        "the two INSERTs in the batch committed"
    );
    let seeded_one = items.get(1).await.expect("get updated seed row");
    assert_eq!(
        seeded_one.price, 333,
        "the UPDATE in the same batch committed too"
    );

    // All-or-nothing rollback: a batch whose second row UPDATEs a NON-existent
    // id (so `rows_affected == 0`) must roll the WHOLE thing back - the
    // otherwise-valid INSERT before it must NOT land, and `import` returns
    // `Ok(ImportResult { errors })`, not `Err`. NOTE: the trigger has to be a
    // missing-id UPDATE, not a bad-value row - validation runs BEFORE any
    // transaction opens, so a validation error never reaches
    // `import_apply_postgres`'s rollback branch.
    let import_rollback = items
        .import(vec![
            ItemImportRow {
                id: None,
                name: "巻き戻るはず".to_string(),
                price: 999,
                stock: 9,
            },
            ItemImportRow {
                id: Some(10_000_000),
                name: "存在しないid".to_string(),
                price: 1,
                stock: 1,
            },
        ])
        .await
        .expect("items import rollback returns Ok(with errors), never Err");
    assert_eq!(import_rollback.created, 0);
    assert_eq!(import_rollback.updated, 0);
    assert_eq!(
        import_rollback.errors.len(),
        1,
        "only the missing-id row is an error"
    );
    assert_eq!(
        import_rollback.errors[0].row, 1,
        "0-based index of the failing row"
    );
    let after_rollback = items
        .list(ListParams::default())
        .await
        .expect("items list after rollback");
    assert_eq!(
        after_rollback.total_count, 1_002,
        "the valid INSERT in the rolled-back batch must not have landed"
    );

    // --- users: create + list ------------------------------------------------
    let users = UsersService::new(db.clone());
    let identity = users
        .create_user("alice", "correct horse battery", "Alice", Role::Editor)
        .await
        .expect("users create_user");
    assert_eq!(identity.username, "alice");
    let listed_users = users.list_users().await.expect("users list");
    assert!(
        listed_users.iter().any(|u| u.username == "alice"),
        "created user should appear in list"
    );

    // --- settings: set + get -------------------------------------------------
    let settings = SettingsService::new(db.clone());
    settings
        .set("smoke.key", "smoke.value")
        .await
        .expect("settings set");
    let got = settings.get("smoke.key").await.expect("settings get");
    assert_eq!(got.as_deref(), Some("smoke.value"));

    // --- audit: record, list, prune (both branches) --------------------------
    let audit = AuditLogService::new(db.clone());
    for i in 0..3 {
        audit
            .try_record(AuditEntry {
                actor_username: Some("alice"),
                actor_role: Some("editor"),
                action: "create",
                resource: "items",
                entity_id: Some(&i.to_string()),
                detail: None,
                origin: "rest",
                result: "ok",
            })
            .await
            .expect("audit try_record");
    }
    let audit_rows = audit.list(ListParams::default()).await.expect("audit list");
    assert_eq!(audit_rows.total_count, 3, "three audit rows recorded");

    // retention_days > 0 exercises the `ts::timestamptz < NOW() - interval`
    // Postgres path; our entries are "now", so a 1-day cutoff deletes nothing
    // (the point is that the TEXT->timestamptz cast does not error).
    let deleted_by_age = audit
        .prune(Some(1), None)
        .await
        .expect("audit prune by age");
    assert_eq!(deleted_by_age, 0);

    // retention_rows keeps only the newest row, deleting the two oldest by id
    // (id order == insertion order via IDENTITY).
    let deleted_by_rows = audit
        .prune(None, Some(1))
        .await
        .expect("audit prune by rows");
    assert_eq!(deleted_by_rows, 2);
    let remaining = audit
        .list(ListParams::default())
        .await
        .expect("audit list after prune");
    assert_eq!(remaining.total_count, 1);

    // --- backup: SQLite-only, so every op must Err (never panic) on Postgres --
    // V2 owner decision D3 (PR4): backup/restore is a SQLite-only feature.
    // Against a Postgres handle each public operation returns `Err` instead of
    // the old `sqlite_pool()` `expect` panic. The `db_path` here is irrelevant
    // (the backend gate fires before any filesystem access), so a placeholder
    // name is fine.
    let backup = BackupService::new(PathBuf::from("banto-unused.sqlite3"), db.clone());
    assert!(
        backup.create().await.is_err(),
        "backup create must error on Postgres, not panic"
    );
    assert!(
        backup.list().await.is_err(),
        "backup list must error on Postgres"
    );
    assert!(
        backup.read("banto-anything.sqlite3").await.is_err(),
        "backup read must error on Postgres"
    );
    assert!(
        backup
            .stage_restore_from_file("banto-anything.sqlite3")
            .await
            .is_err(),
        "stage_restore_from_file must error on Postgres"
    );
    assert!(
        backup.stage_restore_from_bytes(b"anything").await.is_err(),
        "stage_restore_from_bytes must error on Postgres"
    );
    assert!(
        backup.cancel_pending_restore().await.is_err(),
        "cancel_pending_restore must error on Postgres"
    );
    assert!(
        backup.pending_restore().await.is_none(),
        "pending_restore reports nothing staged on Postgres"
    );

    // --- Business ドメイン: 採算（Phase 4）の集計 SQL を実 PostgreSQL で ---
    // `profitability` は SUM を使う唯一のサービス。PostgreSQL の
    // `SUM(bigint)` は `numeric` を返すため `CAST(... AS BIGINT)` で包んで
    // いるが、それが効いているかは実サーバに当てないと分からない（SQLite
    // では型親和性で通ってしまう）。工数原価・経費の税抜換算まで通しで見る。
    let customers = CustomersService::new(db.clone());
    let customer = customers
        .create(CustomerInput {
            code: "PG-C001".to_string(),
            name: "架空商事".to_string(),
            contact_person: None,
            address: None,
            phone: None,
            email: None,
            billing_name: None,
            closing_day: DAY_END_OF_MONTH,
            payment_month_offset: 1,
            payment_day: DAY_END_OF_MONTH,
            note: None,
        })
        .await
        .expect("customer on postgres");
    let projects = ProjectsService::new(db.clone());
    let project = projects
        .create(ProjectInput {
            code: String::new(),
            customer_id: customer.id,
            name: "架空案件".to_string(),
            status: "IN_PROGRESS".to_string(),
            started_on: None,
            due_on: None,
            estimate_amount: None,
            contract_amount: Some(1_000_000),
            billing_hourly_rate: Some(10_000),
            scope: None,
            note: None,
        })
        .await
        .expect("project on postgres");

    let masters = MastersService::new(db.clone());
    for (code, rate) in [("DESIGN", 6_000), ("TRAVEL", 3_000)] {
        masters
            .set_cost_rate(CostRateInput {
                work_category_code: code.to_string(),
                hourly_rate: rate,
            })
            .await
            .expect("cost rate upsert on postgres");
    }

    let work_logs = WorkLogsService::new(db.clone());
    for (code, minutes) in [("DESIGN", 600), ("TRAVEL", 300)] {
        work_logs
            .create(WorkLogInput {
                project_id: project.id,
                trip_id: None,
                worked_on: "2026-08-20".to_string(),
                work_category_code: code.to_string(),
                minutes,
                applied_rate: None,
                description: None,
                invoiced: false,
            })
            .await
            .expect("work log on postgres");
    }

    let expenses = ExpensesService::new(db.clone());
    expenses
        .create(ExpenseInput {
            project_id: project.id,
            trip_id: None,
            spent_on: "2026-08-20".to_string(),
            expense_category_code: "TRANSPORT".to_string(),
            payee: None,
            amount: 11_000,
            tax_category: Some("STANDARD_10".to_string()),
            description: None,
            billable: true,
            invoiced: false,
        })
        .await
        .expect("expense on postgres");

    let profitability = ProfitabilityService::new(db.clone());
    let result = profitability
        .get(project.id)
        .await
        .expect("profitability on postgres");
    // 設計 600分 × 6,000円/時 = 60,000円 + 移動 300分 × 3,000円/時 = 15,000円
    assert_eq!(result.work_cost, 75_000);
    assert_eq!(result.total_minutes, 900);
    assert_eq!(result.excluded_minutes, 300);
    // 税込 11,000 円 → 税抜 10,000 円（切捨て）
    assert_eq!(result.expense_cost, 10_000);
    assert_eq!(result.billable_expense_cost, 10_000);
    assert_eq!(result.total_cost, 85_000);
    assert_eq!(result.gross_profit, -85_000);
    // 実質時間単価は2種とも返る（要件 F-P2）
    assert_eq!(
        result.effective_rate_including_travel,
        Some(-85_000 * 60 / 900)
    );
    assert_eq!(
        result.effective_rate_excluding_travel,
        Some(-85_000 * 60 / 600)
    );

    // --- Business ドメイン: 請求（Phase 5）の確定・取消を実 PostgreSQL で ---
    // 確定は「請求書の更新 + 税集計の入れ替え + 元データの invoiced 反映」を
    // 1トランザクションで行う。トランザクションの分岐は方言ごとにマクロで
    // 展開しているので、Postgres 側が実際に通ることをここで確認する。
    let issuer = IssuerService::new(SettingsService::new(db.clone()));
    issuer
        .set(IssuerInput {
            name: Some("架空設計事務所".to_string()),
            registration_number: Some("T1234567890123".to_string()),
            address: Some("架空県架空市1-2-3".to_string()),
            bank_account: None,
            rounding_mode: "FLOOR".to_string(),
        })
        .await
        .expect("issuer settings on postgres");

    let invoices = InvoicesService::new(db.clone());
    let draft = invoices
        .create(InvoiceInput {
            customer_id: customer.id,
            closing_on: None,
            due_on: None,
            corrected_invoice_id: None,
            note: None,
            lines: vec![
                InvoiceLineInput {
                    project_id: project.id,
                    item_name: "設計".to_string(),
                    quantity: 3,
                    unit_price: 33_335,
                    tax_category: "STANDARD_10".to_string(),
                    source_type: None,
                    source_id: None,
                    note: None,
                },
                InvoiceLineInput {
                    project_id: project.id,
                    item_name: "交通費".to_string(),
                    quantity: 1,
                    unit_price: 10_000,
                    tax_category: "REDUCED_8".to_string(),
                    source_type: None,
                    source_id: None,
                    note: None,
                },
            ],
        })
        .await
        .expect("invoice draft on postgres");
    assert_eq!(draft.invoice.invoice_number, None);

    let issued = invoices
        .issue(draft.invoice.id)
        .await
        .expect("issue on postgres");
    // 税率区分ごとに1回だけ端数処理する:
    //   10%: floor(100,005 × 10%) = 10,000（行ごとなら 9,999）
    //    8%: floor(10,000 × 8%)   =    800
    assert_eq!(issued.invoice.total_taxable, 110_005);
    assert_eq!(issued.invoice.total_tax, 10_800);
    assert_eq!(issued.tax_summaries.len(), 2);
    assert!(issued
        .invoice
        .invoice_number
        .as_deref()
        .expect("number")
        .starts_with("INV-"));

    // 案件採算の売上は確定済みの明細から集計される（要件 F-P4）。
    let with_revenue = profitability
        .get(project.id)
        .await
        .expect("profitability after issue");
    assert_eq!(with_revenue.revenue, 110_005);

    invoices
        .cancel(issued.invoice.id)
        .await
        .expect("cancel on postgres");
    let after_cancel = profitability
        .get(project.id)
        .await
        .expect("profitability after cancel");
    assert_eq!(after_cancel.revenue, 0);
}
