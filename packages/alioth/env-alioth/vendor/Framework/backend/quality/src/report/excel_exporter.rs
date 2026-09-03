use crate::report::{QualityReport, ReportError};

/// Excel 导出器
pub struct ExcelExporter;

impl ExcelExporter {
    /// 导出报告为 Excel
    pub fn export(report: &QualityReport) -> Result<Vec<u8>, ReportError> {
        // 使用 csv 格式作为简化实现
        // 实际项目中应使用 rust_xlsxwriter 或类似库

        let mut csv_content = String::new();

        // 添加 BOM 标记（UTF-8）
        csv_content.push('\u{FEFF}');

        // 添加报告标题
        csv_content.push_str("数据质量报告\n");
        csv_content.push_str(&format!(
            "生成时间,{}",
            report.generated_at.format("%Y-%m-%d %H:%M:%S")
        ));
        csv_content.push('\n');
        csv_content.push('\n');

        // 添加摘要
        csv_content.push_str("摘要\n");
        csv_content.push_str(&format!("整体评分,{}\n", report.summary.overall_score));
        csv_content.push_str(&format!("总规则数,{}\n", report.summary.total_rules));
        csv_content.push_str(&format!("活跃规则,{}\n", report.summary.active_rules));
        csv_content.push_str(&format!("执行次数,{}\n", report.summary.total_executions));
        csv_content.push_str(&format!("失败次数,{}\n", report.summary.failed_executions));
        csv_content.push_str(&format!("通过率,{:.2}%\n", report.summary.pass_rate));
        csv_content.push_str(&format!(
            "报告期间,{} 至 {}\n",
            report.summary.period_start.format("%Y-%m-%d"),
            report.summary.period_end.format("%Y-%m-%d")
        ));
        csv_content.push('\n');

        // 添加规则详情
        if !report.details.is_empty() {
            csv_content.push_str("规则执行详情\n");
            csv_content.push_str(
                "规则名称,规则类型,严重程度,执行次数,通过次数,失败次数,通过率,最后执行时间\n",
            );

            for detail in &report.details {
                let last_executed = detail
                    .last_executed
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "-".to_string());

                csv_content.push_str(&format!(
                    "{},{},{},{},{},{},{:.2}%,{}\n",
                    Self::escape_csv_field(&detail.rule_name),
                    detail.rule_type,
                    detail.severity,
                    detail.execution_count,
                    detail.passed_count,
                    detail.failed_count,
                    detail.pass_rate,
                    last_executed
                ));
            }
            csv_content.push('\n');
        }

        // 添加改进建议
        if !report.recommendations.is_empty() {
            csv_content.push_str("改进建议\n");
            csv_content.push_str("优先级,分类,标题,描述,影响范围,预计影响\n");

            for rec in &report.recommendations {
                let priority = match rec.priority {
                    crate::report::RecommendationPriority::High => "高",
                    crate::report::RecommendationPriority::Medium => "中",
                    crate::report::RecommendationPriority::Low => "低",
                };

                let affected = if rec.affected_rules.is_empty() {
                    "-".to_string()
                } else {
                    rec.affected_rules.join("; ")
                };

                csv_content.push_str(&format!(
                    "{},{},{},{},{},{}\n",
                    priority,
                    rec.category,
                    Self::escape_csv_field(&rec.title),
                    Self::escape_csv_field(&rec.description),
                    Self::escape_csv_field(&affected),
                    Self::escape_csv_field(&rec.estimated_impact)
                ));
            }
        }

        Ok(csv_content.into_bytes())
    }

    /// 转义 CSV 字段
    fn escape_csv_field(field: &str) -> String {
        if field.contains(',')
            || field.contains('"')
            || field.contains('\n')
            || field.contains('\r')
        {
            format!("\"{}\"", field.replace('"', "\"\""))
        } else {
            field.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_csv_field() {
        assert_eq!(ExcelExporter::escape_csv_field("simple"), "simple");
        assert_eq!(
            ExcelExporter::escape_csv_field("with,comma"),
            "\"with,comma\""
        );
        assert_eq!(
            ExcelExporter::escape_csv_field("with\"quote"),
            "\"with\"\"quote\""
        );
    }
}
