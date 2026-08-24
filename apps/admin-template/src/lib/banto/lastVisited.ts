/**
 * 前回開いていた画面の記憶（`docs/mobile-ui-plan.md` P1-1）。
 *
 * スマホ母艦では最頻操作（クイック入力）が「起動 → dashboard 着地 →
 * ハンバーガー → 項目」と毎回 2 タップ先にある。ログイン成功時に
 * ここで覚えたパスへ直接戻すことで、その固定費を消す。
 *
 * 記憶の粒度は**ナビ項目のセクションパス**（`/expenses/123` → `/expenses`）。
 * 詳細行は次に開くとき消えているかもしれないが、セクションは常に在る。
 *
 * adminOnly のパスは記憶しない —— ログイン成功の瞬間はまだ
 * `sessionStore.load()`（`(app)/+layout.ts`）が走っておらずロールが
 * 分からないので、誰が着地しても安全なパスだけを覚える。admin ページを
 * 使い込む場面は稀で、dashboard へ落ちても失うものは小さい。
 *
 * 保存先はクイック入力の前回値と同じ `UiSettingsProvider`（Tauri は
 * 端末ローカル、LAN ブラウザはユーザーごとにサーバ保存）。
 */
import { navItems } from '$lib/navigation';
import { getUiSettings } from '$lib/banto/setup';

const LAST_PATH_KEY = 'nav.lastPath';

// 同じセクション内の遷移（一覧 → 詳細など）で毎回書かない。
let lastWritten: string | null = null;

/** `pathname` の属するナビ項目を返す（`pageTitle` と同じ前方一致）。 */
function sectionOf(pathname: string) {
	return navItems.find((entry) => pathname === entry.path || pathname.startsWith(entry.path + '/'));
}

/**
 * 現在の画面を記憶する。fire-and-forget —— 覚えられなくても画面遷移は
 * 済んでいるので、失敗は握りつぶす（クイック入力の前回値と同じ扱い）。
 */
export function rememberLastVisited(pathname: string): void {
	const item = sectionOf(pathname);
	if (!item || item.adminOnly) return;
	if (item.path === lastWritten) return;
	lastWritten = item.path;
	getUiSettings()
		.set(LAST_PATH_KEY, item.path)
		.catch(() => {
			// 次に成功したときに上書きされる。
			lastWritten = null;
		});
}

/**
 * 記憶したパスを返す。無い・読めない・現在のナビに無い（画面が消えた等）
 * 場合は null —— 呼び出し側が従来どおり dashboard へ落とす。
 */
export async function lastVisitedPath(): Promise<string | null> {
	try {
		const stored = await getUiSettings().get(LAST_PATH_KEY);
		if (!stored) return null;
		const item = navItems.find((entry) => entry.path === stored);
		return item && !item.adminOnly ? item.path : null;
	} catch {
		return null;
	}
}
