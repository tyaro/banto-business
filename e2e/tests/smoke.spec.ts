/**
 * M18 Phase B smoke E2E (docs/roadmap.md M18, docs/history/improvements.md §4).
 *
 * Runs against a real `banto-serve --features embed-ui` (LAN/REST mode, no
 * mocked DataProvider - see playwright.config.ts's doc comment) with a
 * brand-new SQLite database, so scenario 1 legitimately hits the first-run
 * setup screen. All scenarios share ONE browser page/session and run in
 * file order (`describe.serial` + `workers: 1`, config-wide): later
 * scenarios rely on state earlier ones created (the admin account, the
 * customer, the viewer account, the audit trail, ...), the same way a person
 * clicking through the app once would. This is intentionally NOT a
 * from-scratch-state-per-test suite - keep new scenarios in this ordering
 * discipline rather than trying to make them independent.
 *
 * Deliberately scoped to a smoke pass (one scenario per screen, per roadmap
 * M18's non-scope note) - not exhaustive coverage of any one feature (M14
 * audit log, M16 command palette, M17 backups, M20 attachments already have
 * their own focused unit/integration tests elsewhere).
 *
 * Flakiness: no explicit `waitForTimeout`/`sleep` anywhere in this file -
 * every wait is either Playwright's built-in locator auto-retry
 * (`expect(locator)...`) or a real event (`page.once('dialog', ...)`).
 */
import { expect, test, type Locator, type Page } from '@playwright/test';

const ADMIN_USERNAME = 'e2e-admin';
const ADMIN_PASSWORD = 'E2eAdminPass1';
const ADMIN_DISPLAY_NAME = 'E2E管理者';

const VIEWER_USERNAME = 'e2e-viewer';
const VIEWER_PASSWORD = 'E2eViewerPass1';
const VIEWER_DISPLAY_NAME = 'E2E閲覧者';

// Timestamped so a stray leftover row from an interrupted previous run (this
// suite always starts from a fresh DB, so that shouldn't happen, but the
// name doubling as the grid-filter needle makes it worth being paranoid)
// can never collide with the row this run creates.
const CUSTOMER_CODE = `E2E-${Date.now()}`;
const CUSTOMER_NAME = '架空商事';
const CUSTOMER_NAME_UPDATED = '架空商事（改称後）';

// 領収書の添付シナリオ（要件 F-E3）。経費は案件に、案件は顧客にぶら下がるので
// 専用の顧客→案件→経費を1本作る。scenario 3 の顧客は消えるため使い回さない。
// 顧客コードは20文字以内（`customers.rs` の `MAX_CODE_LEN`）。接頭辞を伸ばすと
// タイムスタンプと合わせて超える。
const ATTACHMENT_CUSTOMER_CODE = `E2E-A${Date.now()}`;
const ATTACHMENT_PROJECT_NAME = `E2E添付テスト案件-${Date.now()}`;
const ATTACHMENT_EXPENSE_PAYEE = `E2E添付テスト支払先-${Date.now()}`;
const PNG_FILE_NAME = 'attachment-test.png';
const PNG_FILE_NAME_2 = 'attachment-test-2.png';
const TXT_FILE_NAME = 'attachment-note.txt';

// Smallest possible valid PNG (1x1, black pixel) inlined as base64 rather
// than a committed binary fixture (spec's unit D guidance: prefer
// `setInputFiles({ name, mimeType, buffer })` over adding a binary to the
// repo) - real bytes so the server's `image::guess_format`/thumbnail
// pipeline (banto-attachments) actually exercises its real decode path,
// not a fake MIME label.
const MIN_PNG_BASE64 =
	'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=';

function minimalPngBuffer(): Buffer {
	return Buffer.from(MIN_PNG_BASE64, 'base64');
}

/** Open a filterable column's header filter and apply a "contains" filter (the default op) with `value`. Mirrors a user clicking the ▾ icon, typing, and clicking 適用 (FilterPopover.svelte). */
async function applyColumnFilter(page: Page, columnHeader: string, value: string): Promise<void> {
	const label = `${columnHeader}の絞り込み`;
	await page.getByRole('button', { name: label }).click();
	const dialog = page.getByRole('dialog', { name: label });
	await dialog.getByPlaceholder('値を入力').fill(value);
	await dialog.getByRole('button', { name: '適用' }).click();
}

/** Reopen a column's filter and clear it (クリア), leaving the grid unfiltered on that column. */
async function clearColumnFilter(page: Page, columnHeader: string): Promise<void> {
	const label = `${columnHeader}の絞り込み`;
	await page.getByRole('button', { name: label }).click();
	await page.getByRole('dialog', { name: label }).getByRole('button', { name: 'クリア' }).click();
}

/** A grid data row (role="row") whose rendered text contains `text` - matches both the header row and data rows structurally, so callers should pass text unique to a data row. */
function rowWithText(page: Page, text: string): Locator {
	return page.getByRole('row').filter({ hasText: text });
}

/**
 * 顧客の新規作成フォームを埋める。締日・支払月・支払日は**必須**なので、
 * コードと名前だけでは保存できない（99 = 末日、Phase 1 決定 C-8）。
 */
async function fillCustomerForm(page: Page, code: string, name: string): Promise<void> {
	await page.getByLabel('顧客コード').fill(code);
	await page.getByLabel('顧客名').fill(name);
	await page.getByLabel('締日').fill('99');
	await page.getByLabel('支払月').fill('1');
	await page.getByLabel('支払日').fill('99');
}

/**
 * グリッドの行から詳細画面を開き、その id を返す。
 *
 * インライン編集できる列を持つグリッド（= editor 以上で開いた業務一覧）では、
 * 単クリックはセル選択で、`onRowClick` は**ダブルクリック**でしか発火しない
 * （BantoGrid.svelte の `handleCellClick` / `handleCellDoubleClick`）。編集可能な
 * セルをダブルクリックすると編集が始まってしまうので、先頭の操作列（空・
 * 編集不可）を狙う。
 */
async function openRowAndGetId(page: Page, text: string, resource: string): Promise<number> {
	await rowWithText(page, text).getByRole('gridcell').first().dblclick();
	await expect(page).toHaveURL(new RegExp(`/${resource}/\\d+$`));
	const id = Number(page.url().split('/').pop());
	expect(Number.isInteger(id)).toBe(true);
	return id;
}

/** Opens the header's user menu and clicks "ログアウト" (Header.svelte moved logout off a bare header button into the shared Menu component - visual-refresh-design.md §8.2). */
async function logout(page: Page): Promise<void> {
	await page.getByRole('button', { name: 'ユーザーメニューを開く' }).click();
	await page.getByRole('menuitem', { name: 'ログアウト' }).click();
}

test.describe.serial('Banto LAN/REST smoke', () => {
	let page: Page;

	test.beforeAll(async ({ browser }) => {
		// This manually-created shared page bypasses the config's `use`
		// context options, so reduced motion must be passed here explicitly.
		// Without it, View Transitions (visual-refresh-design.md §11.1) freeze
		// the OLD page's snapshot for the crossfade after each navigation and
		// locators can pin an element from the outgoing page (e.g. getByLabel
		// substring-matching a grid filter button right after goto /customers/new).
		page = await browser.newPage({ reducedMotion: 'reduce' });
	});

	test.afterAll(async () => {
		// Optional chaining: if beforeAll throws before `page` is assigned, an
		// unguarded page.close() raises a TypeError here that masks the real
		// failure cause (e.g. a missing Playwright browser).
		await page?.close();
	});

	test('1. first-run setup creates the admin account and reaches the dashboard', async () => {
		await page.goto('/login');

		// Fresh DB -> AuthProvider.status() reports uninitialized -> the login
		// page renders the setup form, not the login form (login/+page.svelte).
		await expect(page.getByRole('heading', { name: 'Banto Business' })).toBeVisible();
		await expect(page.getByLabel('表示名')).toBeVisible();

		await page.getByLabel('表示名').fill(ADMIN_DISPLAY_NAME);
		await page.getByLabel('ユーザー名').fill(ADMIN_USERNAME);
		await page.getByLabel('パスワード（8文字以上）').fill(ADMIN_PASSWORD);
		await page.getByLabel('パスワード（確認）').fill(ADMIN_PASSWORD);
		await page.getByRole('button', { name: 'アカウントを作成' }).click();

		await expect(page).toHaveURL(/\/dashboard$/);
		await expect(page.getByRole('heading', { name: 'ダッシュボード' })).toBeVisible();
	});

	test('2. logout returns to the login screen, then login restores the session', async () => {
		await logout(page);
		await expect(page).toHaveURL(/\/login$/);

		await page.getByLabel('ユーザー名').fill(ADMIN_USERNAME);
		await page.getByLabel('パスワード').fill(ADMIN_PASSWORD);
		await page.getByRole('button', { name: 'ログイン' }).click();

		await expect(page).toHaveURL(/\/dashboard$/);
	});

	test('3. customers: create, appears in the grid, edit, delete', async () => {
		await page.goto('/customers');
		await page.getByRole('link', { name: '新規作成' }).click();
		await expect(page).toHaveURL(/\/customers\/new$/);

		await fillCustomerForm(page, CUSTOMER_CODE, CUSTOMER_NAME);
		await page.getByRole('button', { name: '保存' }).click();
		await expect(page).toHaveURL(/\/customers$/);

		// サーバーモードのグリッド。行を探して回るのではなく、実行ごとに
		// 一意なコードで絞り込む。
		await applyColumnFilter(page, '顧客コード', CUSTOMER_CODE);
		await expect(rowWithText(page, CUSTOMER_CODE)).toBeVisible();

		const id = await openRowAndGetId(page, CUSTOMER_CODE, 'customers');
		const detailPath = `/customers/${id}`;

		// 編集: 顧客名を変えて保存し、グリッド経由ではなく URL で開き直して
		// サーバー側に本当に永続化されたことを確かめる。
		await page.getByLabel('顧客名').fill(CUSTOMER_NAME_UPDATED);
		await page.getByRole('button', { name: '保存' }).click();
		await expect(page).toHaveURL(/\/customers$/);

		await page.goto(detailPath);
		await expect(page.getByLabel('顧客名')).toHaveValue(CUSTOMER_NAME_UPDATED);

		// 削除（window.confirm。クリック前に accept を仕込む）。
		page.once('dialog', (dialog) => dialog.accept());
		await page.getByRole('button', { name: '削除' }).click();
		await expect(page).toHaveURL(/\/customers$/);

		await page.goto(detailPath);
		await expect(page.getByText('顧客が見つかりません')).toBeVisible();
	});

	test('4. user management: create a viewer account', async () => {
		await page.goto('/users');

		// Scoped to the create form (not just page.getByLabel(...)): the
		// grid's ユーザー名/表示名 column filter buttons below (aria-label
		// "<列名>の絞り込み") also match those label texts by substring,
		// which makes an unscoped getByLabel ambiguous once the grid is on
		// the same page (users/+page.svelte's "section.create").
		const createForm = page.locator('section.create');
		await createForm.getByLabel('ユーザー名').fill(VIEWER_USERNAME);
		await createForm.getByLabel('パスワード（8文字以上）').fill(VIEWER_PASSWORD);
		await createForm.getByLabel('表示名').fill(VIEWER_DISPLAY_NAME);
		await createForm.getByLabel('ロール').selectOption('viewer');
		await createForm.getByRole('button', { name: '作成' }).click();

		await expect(rowWithText(page, VIEWER_USERNAME)).toBeVisible();
	});

	test('5. viewer role: no admin nav entries, no create button', async () => {
		await logout(page);
		await expect(page).toHaveURL(/\/login$/);

		await page.getByLabel('ユーザー名').fill(VIEWER_USERNAME);
		await page.getByLabel('パスワード').fill(VIEWER_PASSWORD);
		await page.getByRole('button', { name: 'ログイン' }).click();
		await expect(page).toHaveURL(/\/dashboard$/);

		// Sidebar.svelte hides adminOnly nav entries entirely (not just
		// disabled) for non-admin roles.
		await expect(page.getByRole('link', { name: 'ユーザー管理' })).toHaveCount(0);
		await expect(page.getByRole('link', { name: '監査ログ' })).toHaveCount(0);

		await page.goto('/customers');
		await expect(page.getByRole('link', { name: '新規作成' })).toHaveCount(0);
	});

	test('6. admin: audit log shows the login and customers records', async () => {
		await logout(page);
		await expect(page).toHaveURL(/\/login$/);

		await page.getByLabel('ユーザー名').fill(ADMIN_USERNAME);
		await page.getByLabel('パスワード').fill(ADMIN_PASSWORD);
		await page.getByRole('button', { name: 'ログイン' }).click();
		await expect(page).toHaveURL(/\/dashboard$/);

		await page.goto('/audit-log');

		// action is stored/filtered on its raw wire value ('login'); the grid
		// cell renders it through actionLabel() as 'ログイン'.
		await applyColumnFilter(page, 'アクション', 'login');
		await expect(rowWithText(page, 'ログイン').first()).toBeVisible();

		// Filters AND together, so the previous one must be cleared before a
		// resource-only filter is applied, or nothing would match.
		await clearColumnFilter(page, 'アクション');
		await applyColumnFilter(page, 'リソース', 'customers');
		await expect(rowWithText(page, 'customers').first()).toBeVisible();
	});

	test('7. 経費の詳細: 領収書のアップロード・サムネイル・行・削除', async () => {
		// 経費は案件に、案件は顧客にぶら下がる（要件 F-E3）。まず1本作る。
		await page.goto('/customers/new');
		await fillCustomerForm(page, ATTACHMENT_CUSTOMER_CODE, CUSTOMER_NAME);
		await page.getByRole('button', { name: '保存' }).click();
		await expect(page).toHaveURL(/\/customers$/);

		await applyColumnFilter(page, '顧客コード', ATTACHMENT_CUSTOMER_CODE);
		const customerId = await openRowAndGetId(page, ATTACHMENT_CUSTOMER_CODE, 'customers');

		await page.goto('/projects/new');
		await page.getByLabel('顧客').fill(String(customerId));
		await page.getByLabel('案件名').fill(ATTACHMENT_PROJECT_NAME);
		// 状態は必須の select（既定値なし）。
		await page.getByLabel('状態').selectOption({ label: '受注' });
		await page.getByRole('button', { name: '保存' }).click();
		await expect(page).toHaveURL(/\/projects$/);

		await applyColumnFilter(page, '案件名', ATTACHMENT_PROJECT_NAME);
		const projectId = await openRowAndGetId(page, ATTACHMENT_PROJECT_NAME, 'projects');

		await page.goto('/expenses/new');
		await page.getByLabel('案件').fill(String(projectId));
		await page.getByLabel('支出日').fill('2026-08-20');
		await page.getByLabel('分類').fill('TRANSPORT');
		await page.getByLabel('支払先').fill(ATTACHMENT_EXPENSE_PAYEE);
		await page.getByLabel('金額').fill('1200');
		await page.getByRole('button', { name: '保存' }).click();
		await expect(page).toHaveURL(/\/expenses$/);

		await applyColumnFilter(page, '支払先', ATTACHMENT_EXPENSE_PAYEE);
		const expenseId = await openRowAndGetId(page, ATTACHMENT_EXPENSE_PAYEE, 'expenses');
		const expensePath = `/expenses/${expenseId}`;
		await expect(page.getByRole('heading', { name: '添付ファイル' })).toBeVisible();
		await expect(page.getByText('添付ファイルはありません')).toBeVisible();

		const uploadInput = page.getByLabel('添付ファイルをアップロード');

		// 1. 画像はサムネイルのグリッドに `<img>` として並ぶ
		// （AttachmentsPanel.svelte の `grouped.withThumbnail`）。
		await uploadInput.setInputFiles({
			name: PNG_FILE_NAME,
			mimeType: 'image/png',
			buffer: minimalPngBuffer()
		});
		await expect(page.getByRole('img', { name: PNG_FILE_NAME, exact: true })).toBeVisible();

		// 2. 画像以外はファイル行の一覧に、拡張子バッジ付きで並ぶ。
		await uploadInput.setInputFiles({
			name: TXT_FILE_NAME,
			mimeType: 'text/plain',
			buffer: Buffer.from('e2e attachment smoke test\n', 'utf-8')
		});
		const fileRow = page.locator('.file-row').filter({ hasText: TXT_FILE_NAME });
		await expect(fileRow).toBeVisible();
		await expect(fileRow.getByText('TXT', { exact: true })).toBeVisible();

		// 3. テキストだけ消す。ファイル行は空になるがサムネイルは残る。
		page.once('dialog', (dialog) => dialog.accept());
		await fileRow.getByRole('button', { name: '削除' }).click();
		await expect(fileRow).toHaveCount(0);
		await expect(page.getByRole('img', { name: PNG_FILE_NAME, exact: true })).toBeVisible();

		// 4. 画像も消すと空状態の文言へ戻る。
		const thumbTile = page.locator('.thumb-tile').filter({ hasText: PNG_FILE_NAME });
		page.once('dialog', (dialog) => dialog.accept());
		await thumbTile.getByRole('button', { name: '削除' }).click();
		await expect(page.getByText('添付ファイルはありません')).toBeVisible();

		// 5. もう1件アップロードして、**付けたまま経費ごと削除する**。
		// 経費削除時の領収書の掃除（`expenses_delete_body` /
		// `rest::expenses::expenses_delete`）を、添付ゼロの経費でしか
		// 消さない形にしないため。
		await uploadInput.setInputFiles({
			name: PNG_FILE_NAME_2,
			mimeType: 'image/png',
			buffer: minimalPngBuffer()
		});
		await expect(page.getByRole('img', { name: PNG_FILE_NAME_2, exact: true })).toBeVisible();

		// 後片付け（`.form-panel` で絞る - 添付タイル側にも同じ「削除」が
		// あるので、絞らないと曖昧になる）。
		page.once('dialog', (dialog) => dialog.accept());
		await page.locator('.form-panel').getByRole('button', { name: '削除' }).click();
		await expect(page).toHaveURL(/\/expenses$/);

		await page.goto(expensePath);
		await expect(page.getByText('経費が見つかりません')).toBeVisible();
	});

	test('8. command palette: search and navigate to the audit log', async () => {
		await page.goto('/customers');
		// The Ctrl+K listener lives on (app)/+layout.svelte's `<svelte:window>`,
		// which only mounts after the route guard's async work (bantoReady,
		// sessionStore.load()) resolves - later than page.goto()'s "load"
		// event. Wait for a page-specific element first so the keypress below
		// isn't racing that mount.
		await expect(page.getByRole('link', { name: '新規作成' })).toBeVisible();

		await page.keyboard.press('Control+K');
		const search = page.getByPlaceholder('コマンドを検索…');
		await expect(search).toBeVisible();
		await search.fill('監査');
		await search.press('Enter');

		await expect(page).toHaveURL(/\/audit-log$/);
	});

	test('9. settings: switching to the dark theme sets data-theme', async () => {
		await page.goto('/settings');

		// Not .getByLabel(...).check(): the radio inputs here are visually
		// hidden (`.options input { opacity: 0; pointer-events: none }`,
		// settings/+page.svelte) so their own `<label>` is the real click
		// target - clicking it activates the wrapped input via normal
		// label/control association.
		await page
			.getByRole('radiogroup', { name: 'テーマ' })
			.getByText('ダーク', { exact: true })
			.click();
		await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
	});

	test('10. backups: create a backup and see it in the list', async () => {
		await page.goto('/settings');

		const backupRows = page.locator('.backup-list li');
		await expect(backupRows).toHaveCount(0);

		await page.getByRole('button', { name: '今すぐバックアップ' }).click();
		await expect(backupRows).toHaveCount(1);
	});

	// PR-B3 (i18n layer ②, ADR-0005): the settings
	// language picker actually switches the whole UI locale. Deliberately LAST:
	// Paraglide's setLocale() persists the choice to this shared page's
	// localStorage and reloads every screen, so switching to English here can't
	// disturb the Japanese-asserting scenarios above (which run first).
	test('11. settings: the language picker switches the whole UI to English', async () => {
		await page.goto('/settings');

		// The <select> is still labelled in Japanese (表示言語) at this point. Its
		// option labels (日本語 / English) are intentionally NOT translated, so
		// switch by value. selectOption fires the change handler -> setLocale('en')
		// -> a full reload into English (Paraglide's default).
		const languageSelect = page.getByLabel('表示言語');
		await expect(languageSelect).toBeVisible();
		await languageSelect.selectOption('en');

		// After the reload the page renders in English: the page header
		// (nav.settings) and the language card heading (settings.languageHeading)
		// both flip. Asserting a keyed string proves the switch reached the UI,
		// not just localStorage.
		await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
		await expect(page.getByRole('heading', { name: 'Language', exact: true })).toBeVisible();
	});
});
