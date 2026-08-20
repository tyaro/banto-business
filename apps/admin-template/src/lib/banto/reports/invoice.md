# 請求書

**{{ customerLabel }}** 御中

| | |
|---|---|
| 請求書番号 | {{ invoiceNumber }} |
| 取引年月日 | {{ issuedOn }} |
| 締日 | {{ closingOn }} |
| お支払期限 | {{ dueOn }} |

## ご請求金額 {{ totalAmount | yen }}

（税抜合計 {{ totalTaxable | yen }} / 消費税 {{ totalTax | yen }}）

## 明細

| 品目 | 数量 | 単価 | 金額 | 税率 |
|---|---:|---:|---:|---|
{{#each lines}}
| {{ itemName }}{{ reducedMark }} | {{ quantity | number }} | {{ unitPrice | yen }} | {{ amount | yen }} | {{ taxLabel }} |
{{/each}}

## 税率ごとの内訳

| 適用税率 | 対価の額（税抜） | 消費税額 |
|---|---:|---:|
{{#each taxSummaries}}
| {{ taxLabel }} | {{ taxableAmount | yen }} | {{ taxAmount | yen }} |
{{/each}}

{{#if hasReduced}}
※ は軽減税率（8%）対象品目です。
{{/if}}

## 発行者

{{ issuerName }}

登録番号 {{ issuerRegistrationNumber }}

{{ issuerAddress }}

{{#if bankAccount}}
お振込先: {{ bankAccount }}
{{/if}}

{{#if note}}
備考: {{ note }}
{{/if}}
