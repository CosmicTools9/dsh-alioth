//! 业务审计导出 CSV 组装（纯函数面——行结构来自 ns 服务 pub Row，无第二查询）。

#[cfg(any(feature = "wz", test))]
use audit_writer::read::csv_field;

/// 进项发票导出（WZ `accounts-payable::InvoiceInRow`）。
#[cfg(feature = "wz")]
pub fn invoice_in_csv(rows: &[wz_service_accounts_payable::models::InvoiceInRow]) -> String {
    let mut csv =
        String::from("id,invoice_no,invoice_type,vendor,buyer,amount,status,created_at\n");
    for r in rows {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            r.id,
            csv_field(r.invoice_no.as_deref().unwrap_or("")),
            csv_field(r.invoice_type.as_deref().unwrap_or("")),
            csv_field(r.vendor.as_deref().unwrap_or("")),
            csv_field(r.buyer.as_deref().unwrap_or("")),
            csv_field(r.amount_display.as_deref().unwrap_or("")),
            csv_field(r.status.as_deref().unwrap_or("")),
            csv_field(&r.created_at.map(|t| t.to_rfc3339()).unwrap_or_default()),
        ));
    }
    csv
}

/// 银行回单导出（WZ `accounts-receivable::ReceiptRow`）。
#[cfg(feature = "wz")]
pub fn receipt_csv(rows: &[wz_service_accounts_receivable::models::ReceiptRow]) -> String {
    let mut csv =
        String::from("id,code,notice,amount,status,period,org,counterparty,account_no,creator\n");
    for r in rows {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            r.id,
            csv_field(r.code.as_deref().unwrap_or("")),
            csv_field(r.notice.as_deref().unwrap_or("")),
            csv_field(r.amount.as_deref().unwrap_or("")),
            csv_field(r.status_code.as_deref().unwrap_or("")),
            csv_field(r.period_display.as_deref().unwrap_or("")),
            csv_field(r.org.as_deref().unwrap_or("")),
            csv_field(r.counterparty.as_deref().unwrap_or("")),
            csv_field(r.account_no.as_deref().unwrap_or("")),
            csv_field(r.creator.as_deref().unwrap_or("")),
        ));
    }
    csv
}

/// 付款核销导出（WZ `accounts-payable::PaymentMatchRow`）。
#[cfg(feature = "wz")]
pub fn payment_match_csv(rows: &[wz_service_accounts_payable::models::PaymentMatchRow]) -> String {
    let mut csv =
        String::from("id,bill_no,vendor,payment_no,verify_amount,verify_date,created_at\n");
    for r in rows {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            r.id,
            csv_field(r.bill_no.as_deref().unwrap_or("")),
            csv_field(r.vendor.as_deref().unwrap_or("")),
            csv_field(r.payment_no.as_deref().unwrap_or("")),
            csv_field(r.verify_amount.as_deref().unwrap_or("")),
            csv_field(r.verify_date.as_deref().unwrap_or("")),
            csv_field(&r.created_at.map(|t| t.to_rfc3339()).unwrap_or_default()),
        ));
    }
    csv
}

#[cfg(test)]
mod tests {
    use super::csv_field;

    #[test]
    fn csv_field_escapes_comma_quote_newline() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("l1\nl2"), "\"l1\nl2\"");
    }
}
