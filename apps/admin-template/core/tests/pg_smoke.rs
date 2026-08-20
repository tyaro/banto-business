//! App-layer PostgreSQL smoke test (V2 "PostgreSQL アプリ全体対応", PR3).
//!
//! This is the first place the whole app service stack is exercised against a
//! real PostgreSQL server: `init_db_from_target` runs `migrations-postgres`,
//! then the app services (Business ドメイン一式と `users`/`settings`/
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
use admin_template_core::calendar::CalendarService;
use admin_template_core::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
use admin_template_core::db::init_db_from_target;
use admin_template_core::expenses::{ExpenseInput, ExpensesService};
use admin_template_core::invoices::{InvoiceInput, InvoiceLineInput, InvoicesService};
use admin_template_core::issuer::{IssuerInput, IssuerService};
use admin_template_core::masters::{CostRateInput, MastersService};
use admin_template_core::payments::{PaymentAllocationInput, PaymentInput, PaymentsService};
use admin_template_core::profitability::ProfitabilityService;
use admin_template_core::projects::{ProjectInput, ProjectsService};
use admin_template_core::settings::SettingsService;
use admin_template_core::users::{Role, UsersService};
use admin_template_core::work_logs::{WorkLogInput, WorkLogsService};
use banto_core::ListParams;
use std::path::PathBuf;

/// pg_smoke の各テストは**同じ PostgreSQL データベース**を共有し、それぞれ
/// [`reset_schema`] でスキーマを作り直す。`cargo test` は既定でテストを並列に
/// 走らせるので、直列化しないと互いのテーブルを消し合う（片方が
/// `DROP TABLE` した直後にもう片方が INSERT する）。
///
/// テストが1本だけの間は表面化しなかったが、Phase 8 で2本目・3本目を足した
/// 時点で実際に落ちた。依存を増やさない（`ADR-0002`）ため、既に入っている
/// tokio の Mutex で順番待ちにする。`std::sync::Mutex` は await をまたいで
/// 保持できない。
static PG_SMOKE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
        "DROP TABLE IF EXISTS payment_allocations, payments, invoice_tax_summaries, \
         invoice_lines, invoices, expenses, work_logs, trips, cost_rates, work_categories, \
         expense_categories, projects, customers CASCADE",
        // Phase 8（同期の土台）。
        "DROP TABLE IF EXISTS sync_outbox, sync_state CASCADE",
        "DROP TABLE IF EXISTS _sqlx_migrations",
    ] {
        sqlx::query(stmt)
            .execute(&pool)
            .await
            .expect("drop for reset");
    }
    pool.close().await;
}

/// 採番レンジの確認だけに使う最小の顧客入力。
fn range_customer(code: &str, name: &str) -> CustomerInput {
    CustomerInput {
        code: code.to_string(),
        name: name.to_string(),
        contact_person: None,
        address: None,
        phone: None,
        email: None,
        billing_name: None,
        closing_day: DAY_END_OF_MONTH,
        payment_month_offset: 1,
        payment_day: DAY_END_OF_MONTH,
        note: None,
    }
}

#[tokio::test]
async fn app_layer_crud_round_trips_on_postgres() {
    let Ok(url) = std::env::var("BANTO_TEST_PG_URL") else {
        eprintln!("pg_smoke: BANTO_TEST_PG_URL unset - skipping (no PostgreSQL server)");
        return;
    };
    // 同じ DB を共有するので直列化する（`PG_SMOKE_LOCK` の doc を参照）。
    let _serialized = PG_SMOKE_LOCK.lock().await;

    reset_schema(&url).await;

    let db = init_db_from_target(&url)
        .await
        .expect("init_db_from_target should run migrations-postgres");

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
                resource: "customers",
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

    // --- Business ドメイン: 入金・消込（Phase 6）を実 PostgreSQL で ---
    // 残額・入金状態・期限超過は列に持たず、相関サブクエリの SUM から導出する。
    // ここも `CAST(... AS BIGINT)` が要る箇所なので、実サーバで確認する。
    let payments = PaymentsService::new(db.clone());
    let before = payments
        .settlement(issued.invoice.id)
        .await
        .expect("settlement on postgres");
    assert_eq!(before.total_amount, 120_805);
    assert_eq!(before.remaining_amount, 120_805);
    assert_eq!(before.settlement_status, "ISSUED");

    // 先方が手数料 660 円を差し引いて入金した場合（決定 C-19）。
    payments
        .create(PaymentInput {
            customer_id: customer.id,
            paid_on: "2026-09-30".to_string(),
            amount: 120_145,
            method: Some("振込".to_string()),
            note: None,
            allocations: vec![PaymentAllocationInput {
                invoice_id: issued.invoice.id,
                allocated_amount: 120_145,
                difference_reason: Some("TRANSFER_FEE".to_string()),
                difference_amount: 660,
                note: None,
            }],
        })
        .await
        .expect("payment on postgres");

    let settled = payments
        .settlement(issued.invoice.id)
        .await
        .expect("settlement after payment");
    assert_eq!(settled.settled_amount, 120_805);
    assert_eq!(settled.remaining_amount, 0);
    assert_eq!(settled.settlement_status, "PAID");
    // 完済したので未入金一覧には出ない（要件 F-Y7）。
    let outstanding = payments
        .outstanding()
        .await
        .expect("outstanding on postgres");
    assert!(outstanding
        .iter()
        .all(|s| s.invoice_id != issued.invoice.id));

    // --- 月カレンダー（Phase 7 準備）の集計 SQL を実 PostgreSQL で ---
    // `profitability` と同じ理由でここに置く。カレンダーは 5 テーブルに
    // またがって `SUM`/`COUNT` を掛けるので、`CAST(... AS BIGINT)` の抜けが
    // あれば必ずここで落ちる。日付の閉区間（月初・月末）も実サーバの
    // TEXT 比較で確かめる。
    let calendar = CalendarService::new(db.clone());
    let august = calendar
        .month("2026-08")
        .await
        .expect("calendar on postgres");
    let worked_day = august
        .iter()
        .find(|day| day.date == "2026-08-20")
        .expect("2026-08-20 should carry the work logs and the expense");
    assert_eq!(worked_day.worked_minutes, 900); // 600 + 300
    assert_eq!(worked_day.work_log_count, 2);
    assert_eq!(worked_day.projects.len(), 1);
    assert_eq!(worked_day.projects[0].minutes, 900);
    assert_eq!(worked_day.expense_count, 1);
    assert_eq!(worked_day.expense_amount, 11_000); // 税込のまま

    // 入金は 2026-09-30。月をまたいで正しく振り分けられているか。
    let september = calendar
        .month("2026-09")
        .await
        .expect("calendar on postgres");
    let paid_day = september
        .iter()
        .find(|day| day.date == "2026-09-30")
        .expect("2026-09-30 should carry the payment");
    assert_eq!(paid_day.payment_count, 1);
    assert!(paid_day.payment_amount > 0);
    assert!(
        august.iter().all(|day| day.payment_count == 0),
        "the September payment must not leak into August"
    );

    // 月として読めない指定はエラー（空の月と区別する）。
    assert!(
        calendar.month("2026-13").await.is_err(),
        "a malformed month must be rejected on postgres too"
    );

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

/// **Phase 8: 採番レンジの PostgreSQL 経路。**
///
/// SQLite は `sqlite_sequence` を直接書き換えるが、PostgreSQL は IDENTITY 列の
/// シーケンスを `setval` で動かす。両者は別のコードパスで、**実際の PostgreSQL
/// でしか踏めない**（`pg_get_serial_sequence` の解決も含めて）。
#[tokio::test]
async fn the_device_id_range_applies_to_postgres_identity_sequences() {
    let Ok(url) = std::env::var("BANTO_TEST_PG_URL") else {
        eprintln!("pg_smoke: BANTO_TEST_PG_URL unset - skipping (no PostgreSQL server)");
        return;
    };
    // 同じ DB を共有するので直列化する（`PG_SMOKE_LOCK` の doc を参照）。
    let _serialized = PG_SMOKE_LOCK.lock().await;

    reset_schema(&url).await;
    let db = init_db_from_target(&url)
        .await
        .expect("init_db_from_target should succeed");

    // デバイス 1（Pixel）のレンジへ寄せる。
    admin_template_core::sync::ensure_id_range(&db, 1)
        .await
        .expect("ensure_id_range on postgres");

    let customers = CustomersService::new(db.clone());
    let first = customers
        .create(range_customer("PG-R001", "架空商事"))
        .await
        .expect("customer on postgres");
    assert_eq!(
        admin_template_core::sync::owning_device(first.id),
        1,
        "id {} がデバイス 1 のレンジに入っていない",
        first.id
    );
    assert!(first.id >= 1_000_000_000);

    // 再実行しても巻き戻らない（同期のたびに呼ばれても安全）。
    admin_template_core::sync::ensure_id_range(&db, 1)
        .await
        .expect("ensure_id_range is idempotent");
    let second = customers
        .create(range_customer("PG-R002", "架空工業"))
        .await
        .expect("second customer");
    assert_eq!(second.id, first.id + 1);
}

/// **Phase 8: 論理削除の PostgreSQL 経路。**
///
/// `deleted_at` は TEXT 列（SQLite に日時型が無いので全ての日時が TEXT）。
/// PostgreSQL 側は `NOW()::text` と明示的に落とす必要があり、`Dialect::now_expr()`
/// の `NOW()`（timestamptz）をそのまま入れると型エラーになる。
///
/// **この経路は実際の PostgreSQL でしか踏めない。** ここに来る前、pg_smoke は
/// 一度も `delete` を呼んでおらず、SQLite だけ通って PostgreSQL で落ちる状態に
/// 気付けなかった。
#[tokio::test]
async fn soft_delete_round_trips_on_postgres() {
    let Ok(url) = std::env::var("BANTO_TEST_PG_URL") else {
        eprintln!("pg_smoke: BANTO_TEST_PG_URL unset - skipping (no PostgreSQL server)");
        return;
    };
    // 同じ DB を共有するので直列化する（`PG_SMOKE_LOCK` の doc を参照）。
    let _serialized = PG_SMOKE_LOCK.lock().await;

    reset_schema(&url).await;
    let db = init_db_from_target(&url)
        .await
        .expect("init_db_from_target should succeed");

    let customer = CustomersService::new(db.clone())
        .create(range_customer("PG-D001", "架空商事"))
        .await
        .expect("customer");
    let project = ProjectsService::new(db.clone())
        .create(ProjectInput {
            code: "PG-P001".to_string(),
            customer_id: customer.id,
            name: "架空案件".to_string(),
            status: "IN_PROGRESS".to_string(),
            started_on: None,
            due_on: None,
            estimate_amount: None,
            contract_amount: None,
            billing_hourly_rate: None,
            scope: None,
            note: None,
        })
        .await
        .expect("project");

    let work_logs = WorkLogsService::new(db.clone());
    let keep = work_logs
        .create(WorkLogInput {
            project_id: project.id,
            trip_id: None,
            worked_on: "2026-08-20".to_string(),
            work_category_code: "DESIGN".to_string(),
            minutes: 60,
            // このテストの主題は削除経路で原価ではないので、レートマスタに
            // 依存せず単価を直接渡す。
            applied_rate: Some(6_000),
            description: None,
            invoiced: false,
        })
        .await
        .expect("残す工数");
    let doomed = work_logs
        .create(WorkLogInput {
            project_id: project.id,
            trip_id: None,
            worked_on: "2026-08-21".to_string(),
            work_category_code: "DESIGN".to_string(),
            minutes: 120,
            applied_rate: Some(6_000),
            description: None,
            invoiced: false,
        })
        .await
        .expect("消す工数");

    work_logs
        .delete(doomed.id)
        .await
        .expect("soft delete on postgres");

    // 墓石は get にも一覧にも出ない。
    assert!(work_logs.get(doomed.id).await.is_err());
    let listed = work_logs
        .list(ListParams::default())
        .await
        .expect("list on postgres");
    assert_eq!(listed.total_count, 1);
    assert_eq!(listed.rows[0].id, keep.id);

    // 採算からも落ちている。
    let profit = ProfitabilityService::new(db.clone())
        .get(project.id)
        .await
        .expect("profitability on postgres");
    assert_eq!(profit.total_minutes, 60);

    // 二重削除は NotFound。
    assert!(work_logs.delete(doomed.id).await.is_err());
}
