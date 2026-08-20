/**
 * 請求の「CRUD に収まらない」操作と、発行者情報のクライアント。
 *
 * 一覧・取得・作成・更新・削除は `@banto/admin-core` の DataProvider
 * （`invoices_list` / `_get` / `_create` / `_update` / `_delete`）でそのまま
 * 通るので、ここには**入らない**。DataProvider の契約に載らないのは
 *
 * - 確定 / 取消（`POST /api/invoices/{id}/issue` / `/cancel`。id と操作だけで
 *   values が無い）
 * - 明細候補（`POST /api/invoices/candidates`。resource CRUD ではない検索）
 * - 発行者情報（単一の設定であり、id を持つリソースではない）
 *
 * の4つ。`usersAdmin.ts` と同じく、パッケージ側の契約を曲げずに小さな専用
 * クライアントを置く（docs/conventions.md §4/§5、CLAUDE.md 第2章）。
 * Tauri は `invoke()`、LAN ブラウザは `fetch()` で、どちらも同じサービス層に
 * 当たる（conventions §1）。
 */
import { invoke } from '@tauri-apps/api/core';
import { getAuthProvider, isProviderError, ProviderError, type ErrorBody } from '@banto/admin-core';
import { CSRF_HEADER, getBantoMode } from './setup';

/** `admin_template_core::invoices::Invoice`（camelCase）。 */
export interface Invoice {
	id: number;
	invoiceNumber: string | null;
	customerId: number;
	status: 'DRAFT' | 'ISSUED' | 'CANCELLED';
	issuedOn: string | null;
	closingOn: string | null;
	dueOn: string | null;
	correctedInvoiceId: number | null;
	totalTaxable: number;
	totalTax: number;
	totalAmount: number;
	roundingMode: string;
	issuerName: string | null;
	issuerRegistrationNumber: string | null;
	issuerAddress: string | null;
	note: string | null;
	createdAt: string;
	updatedAt: string;
}

export interface InvoiceLine {
	id: number;
	invoiceId: number;
	projectId: number;
	lineNo: number;
	itemName: string;
	quantity: number;
	unitPrice: number;
	amount: number;
	taxCategory: string;
	sourceType: string | null;
	sourceId: number | null;
	note: string | null;
}

export interface InvoiceTaxSummary {
	id: number;
	invoiceId: number;
	taxCategory: string;
	rateBp: number;
	taxableAmount: number;
	taxAmount: number;
}

/** `getOne('invoices', id)` の戻り値でもある（`InvoiceDetail` は Invoice を flatten している）。 */
export interface InvoiceDetail extends Invoice {
	customerName: string;
	customerBillingName: string | null;
	lines: InvoiceLine[];
	taxSummaries: InvoiceTaxSummary[];
}

export interface InvoiceLineInput {
	projectId: number;
	itemName: string;
	quantity: number;
	unitPrice: number;
	taxCategory: string;
	sourceType?: string | null;
	sourceId?: number | null;
	note?: string | null;
}

export interface InvoiceInput {
	customerId: number;
	closingOn?: string | null;
	dueOn?: string | null;
	correctedInvoiceId?: number | null;
	note?: string | null;
	lines: InvoiceLineInput[];
}

export interface CandidateLine {
	projectId: number;
	projectCode: string;
	projectName: string;
	sourceType: string;
	sourceId: number;
	itemName: string;
	quantity: number;
	unitPrice: number;
	amount: number;
	taxCategory: string;
	note: string;
	billingHourlyRate: number | null;
	minutes: number | null;
}

export interface IssuerSettings {
	name: string | null;
	registrationNumber: string | null;
	address: string | null;
	bankAccount: string | null;
	roundingMode: 'FLOOR' | 'ROUND' | 'CEIL';
}

export const DEMO_MODE_MESSAGE = 'デモモードでは利用できません';

function demoModeError(): ProviderError {
	return new ProviderError({ kind: 'other', message: DEMO_MODE_MESSAGE });
}

/** 実データを持つ環境か（Tauri か埋め込みサーバー）。ブラウザ単体のデモは false。 */
export function isInvoicesAdminAvailable(): boolean {
	return getBantoMode() !== 'demo';
}

const ERROR_KINDS = new Set([
	'not_found',
	'validation',
	'bad_request',
	'unauthorized',
	'forbidden',
	'storage',
	'other'
]);

function isErrorBody(value: unknown): value is ErrorBody {
	if (typeof value !== 'object' || value === null) return false;
	const kind = (value as { kind?: unknown }).kind;
	return typeof kind === 'string' && ERROR_KINDS.has(kind);
}

function toProviderError(err: unknown): ProviderError {
	if (isProviderError(err)) return err;
	if (isErrorBody(err)) return new ProviderError(err);
	const message = err instanceof Error ? err.message : String(err);
	return new ProviderError({ kind: 'other', message });
}

async function invokeCommand<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
	try {
		return (await invoke(cmd, args)) as T;
	} catch (err) {
		throw toProviderError(err);
	}
}

const NETWORK_ERROR_MESSAGE = 'サーバーに接続できません';

function currentToken(): string | null {
	const auth = getAuthProvider() as { getToken?: () => string | null };
	return auth.getToken ? auth.getToken() : null;
}

async function httpRequest<T>(path: string, method: string, body?: unknown): Promise<T> {
	const hasBody = body !== undefined;
	const headers: Record<string, string> = { ...CSRF_HEADER };
	if (hasBody) headers['Content-Type'] = 'application/json';
	const token = currentToken();
	if (token) headers.Authorization = `Bearer ${token}`;

	let response: Response;
	try {
		response = await fetch(path, {
			method,
			headers,
			body: hasBody ? JSON.stringify(body) : undefined
		});
	} catch {
		throw new ProviderError({ kind: 'other', message: NETWORK_ERROR_MESSAGE });
	}

	if (!response.ok) {
		let errorBody: unknown;
		try {
			errorBody = await response.json();
		} catch {
			throw new ProviderError({
				kind: 'other',
				message: `${response.status} ${response.statusText}`
			});
		}
		if (isErrorBody(errorBody)) throw new ProviderError(errorBody);
		throw new ProviderError({
			kind: 'other',
			message: `${response.status} ${response.statusText}`
		});
	}
	return (await response.json()) as T;
}

/** 未請求の工数・経費から明細候補を作る（要件 F-I1）。 */
export async function invoiceCandidates(
	customerId: number,
	from: string,
	to: string
): Promise<CandidateLine[]> {
	if (!isInvoicesAdminAvailable()) throw demoModeError();
	const query = { customerId, from, to };
	if (getBantoMode() === 'tauri') {
		return invokeCommand<CandidateLine[]>('invoices_candidates', { query });
	}
	return httpRequest<CandidateLine[]>('/api/invoices/candidates', 'POST', query);
}

/** 確定（要件 F-I7）。請求書番号の採番と各種スナップショットはサーバ側で行う。 */
export async function issueInvoice(id: number): Promise<InvoiceDetail> {
	if (!isInvoicesAdminAvailable()) throw demoModeError();
	if (getBantoMode() === 'tauri') {
		return invokeCommand<InvoiceDetail>('invoices_issue', { id });
	}
	return httpRequest<InvoiceDetail>(`/api/invoices/${id}/issue`, 'POST', {});
}

/** 取消（赤伝。決定 C-10）。 */
export async function cancelInvoice(id: number): Promise<InvoiceDetail> {
	if (!isInvoicesAdminAvailable()) throw demoModeError();
	if (getBantoMode() === 'tauri') {
		return invokeCommand<InvoiceDetail>('invoices_cancel', { id });
	}
	return httpRequest<InvoiceDetail>(`/api/invoices/${id}/cancel`, 'POST', {});
}

export async function getIssuerSettings(): Promise<IssuerSettings> {
	if (!isInvoicesAdminAvailable()) throw demoModeError();
	if (getBantoMode() === 'tauri') return invokeCommand<IssuerSettings>('issuer_get');
	return httpRequest<IssuerSettings>('/api/issuer', 'GET');
}

export async function updateIssuerSettings(input: IssuerSettings): Promise<IssuerSettings> {
	if (!isInvoicesAdminAvailable()) throw demoModeError();
	if (getBantoMode() === 'tauri') {
		return invokeCommand<IssuerSettings>('issuer_update', { input });
	}
	return httpRequest<IssuerSettings>('/api/issuer', 'PUT', input);
}
