//! 文件存储公共服务（namespace 级跨应用基础设施）。
//!
//! Gateway 统一入口 `/api/files`（`/api` scope 内，继承 NgacEnforcer PEP：
//! JWT 校验 + NGAC PDP；`files:{id}` 实例级资源）。
//!
//! - 存储空间按 namespace 隔离：`X-Namespace` header（`^[A-Z][a-zA-Z0-9-]*$`）
//!   + 字节目录 `{root}/{ns}/…` + 列表/下载链校验。
//! - 后端可插拔（local/s3/oss）：配置入口在 `isahl.zc_id_prot-oss_config`
//!   （settings 内嵌 enabled/is_default 标志；敏感凭证存 enc_fields），
//!   启动时读取启用配置构造 `FileManager`（scheme 路由）。
//!   文件夹路径取配置级 `isahl.zc_id_stor-plc-path`（fk_config 关联），
//!   缺省回退 `settings.base_path` / 固定默认 `./data/local-files`。
//!   配置唯一真相源为 isahl 表（`zc_id_prot-oss_config`）——无环境变量配置路径。
//! - 行级授权：`created_by_id = $user OR $user = ANY(ak_access_user|ak_permit_user|ak_benefit_user)`。
//! - 完整性（零 DDL）：SHA-256 存 `zc_id_info-url.notice`（`sha256:{hex}`），
//!   下载校验 + ETag/X-Checksum-Sha256 响应头。
//! - 限速：`per_user_any(["/api/files"], 20, 20/60)`（main.rs，SECURITY_SPEC §4）。

use std::sync::Arc;

use framework_file_manager::repository::SqlxFileRepository;
use framework_file_manager::{FileManager, FileService};
use serde_json::Value;
use sqlx::PgPool;

mod handlers;

pub use handlers::{
    configure_routes, delete_file, download_file, list_files, presigned_file, update_file,
    upload_file,
};

/// Gateway 文件服务状态（启动时构造，app_data 注入）。
#[derive(Clone)]
pub struct FilesState {
    pub service: FileService,
    pool: PgPool,
}

/// 活库存储配置行（`isahl.zc_id_prot-oss_config` 投影）。
/// provider 读 `settings->>'provider'`（`_t_` 是 lifecycle 自动维度列，业务禁止使用）。
#[derive(Debug, sqlx::FromRow)]
struct StorageConfigRow {
    id: i64,
    code: Option<String>,
    provider: Option<String>,
    settings: Option<Value>,
    enc_fields: Option<Value>,
}

impl FilesState {
    /// 供 handler 直接查询。
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// 从活库构造：读取 `isahl.zc_id_prot-oss_config` 启用配置（settings 内嵌
    /// enabled/is_default）→ 后端映射 → FileManager（scheme 路由）。
    /// 无任何 storage 配置 → 兜底本地磁盘。
    pub async fn from_live_db(pool: PgPool) -> Self {
        let rows = sqlx::query_as::<_, StorageConfigRow>(
            r#"
            SELECT id, code, settings->>'provider' AS provider, settings, enc_fields
            FROM isahl."zc_id_prot-oss_config"
            WHERE deleted_at IS NULL
            ORDER BY updated_at DESC
            "#,
        )
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

        let mut backends: Vec<(
            String,
            Arc<dyn framework_file_manager::backend::StorageBackend>,
        )> = Vec::new();
        let mut default_scheme: Option<String> = None;
        let mut default_override = false;

        for row in rows {
            // settings 内嵌 enabled 标志（缺省视为未启用）
            let enabled = row
                .settings
                .as_ref()
                .and_then(|s| s.get("enabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if !enabled {
                continue;
            }
            let Some(provider) = row.provider.clone() else {
                continue;
            };
            match build_backend(&provider, &row, &pool).await {
                Ok((scheme, backend)) => {
                    if default_scheme.is_none() {
                        default_scheme = Some(scheme.clone());
                    }
                    // settings 内嵌 is_default 覆盖默认
                    let is_default = row
                        .settings
                        .as_ref()
                        .and_then(|s| s.get("is_default"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if is_default && !default_override {
                        default_scheme = Some(scheme.clone());
                        default_override = true;
                    }
                    backends.push((scheme, backend));
                }
                Err(e) => {
                    common::telemetry::warn!(
                        "files: storage config {} ({provider}) 构造失败: {e}",
                        row.code.clone().unwrap_or_default()
                    );
                }
            }
        }

        // 兜底：无可用 storage 配置 → 本地磁盘（seed 保证存在，此处防御；
        // 固定默认路径，配置唯一真相源为 isahl 表，不读环境变量）
        let file_manager = if backends.is_empty() {
            FileManager::local_only("./data/local-files", "/files")
        } else {
            FileManager::new(default_scheme.unwrap_or_else(|| "local".into()), backends)
        };

        let service = FileService::new(
            file_manager.default_backend(),
            Box::new(SqlxFileRepository::new(pool.clone())),
            pool.clone(),
            Arc::new(file_manager),
        );
        Self { service, pool }
    }
}

/// 配置级文件夹路径：`zc_id_stor-plc-path`（fk_config → oss_config.id）。
/// 路径值存 `url` 列（物理列名沿用）。无记录 → None（调用方回退）。
async fn folder_path_for(pool: &PgPool, config_id: i64) -> Option<String> {
    sqlx::query_scalar(
        r#"
        SELECT url
        FROM isahl."zc_id_stor-plc-path"
        WHERE fk_config = $1 AND deleted_at IS NULL
        ORDER BY id DESC LIMIT 1
        "#,
    )
    .bind(config_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 解密 enc_fields 中的敏感凭证（`enc:` 前缀 AES-256-GCM 密文）。
/// 明文直写（seed 未加密）时原样返回——兼容两种写入路径。
fn decrypt_credential(value: &str) -> String {
    if let Some(payload) = value.strip_prefix("enc:") {
        system_config::crypto::decrypt(payload).unwrap_or_else(|e| {
            common::telemetry::warn!("files: 凭证解密失败（按明文处理）: {e}");
            value.to_string()
        })
    } else {
        value.to_string()
    }
}

/// provider 语义纯函数：scheme（info-url 下载路由键）/默认 region/path-style
/// 默认/endpoint 模板（`{region}` 占位；None = endpoint 必填）。
/// 全部经 S3 兼容协议适配（阿里云 OSS / 腾讯 COS / 华为 OBS / MinIO 均提供
/// S3 兼容端点，SigV4 通用签名，零厂商 SDK 依赖）。
struct ProviderSpec {
    scheme: &'static str,
    default_region: &'static str,
    default_path_style: bool,
    endpoint_template: Option<&'static str>,
}

fn provider_defaults(provider: &str) -> Option<ProviderSpec> {
    match provider {
        "s3" => Some(ProviderSpec {
            scheme: "s3",
            default_region: "us-east-1",
            default_path_style: false,
            endpoint_template: None,
        }),
        "aliyun_oss" => Some(ProviderSpec {
            scheme: "oss",
            default_region: "oss-cn-hangzhou",
            default_path_style: false,
            endpoint_template: Some("{region}.aliyuncs.com"),
        }),
        "tencent_cos" => Some(ProviderSpec {
            scheme: "cos",
            default_region: "ap-guangzhou",
            default_path_style: false,
            endpoint_template: Some("cos.{region}.myqcloud.com"),
        }),
        "huawei_obs" => Some(ProviderSpec {
            scheme: "obs",
            default_region: "cn-north-4",
            default_path_style: false,
            endpoint_template: Some("obs.{region}.myhuaweicloud.com"),
        }),
        "minio" => Some(ProviderSpec {
            scheme: "minio",
            default_region: "us-east-1",
            // 自建 S3 惯例：path-style 寻址（无 bucket 子域）
            default_path_style: true,
            endpoint_template: None,
        }),
        _ => None,
    }
}

/// endpoint 解析：显式 settings.endpoint 优先；缺省按 provider 模板从 region 推导
/// （region 缺省 → provider 默认）；无模板且未配置 → 诚实报错。
fn resolve_endpoint(spec: &ProviderSpec, settings: &serde_json::Value) -> Result<String, String> {
    if let Some(e) = settings
        .get("endpoint")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Ok(e.to_string());
    }
    match spec.endpoint_template {
        Some(tpl) => {
            let region = settings
                .get("region")
                .and_then(|v| v.as_str())
                .unwrap_or(spec.default_region);
            Ok(tpl.replace("{region}", region))
        }
        None => Err("endpoint 缺失".into()),
    }
}

/// 按 provider 构造后端：local / s3 / aliyun_oss / tencent_cos / huawei_obs / minio
/// （云厂商统一 S3 兼容协议）。
/// settings 存非敏感配置；敏感凭证（access_key/secret_key）存 enc_fields。
async fn build_backend(
    provider: &str,
    row: &StorageConfigRow,
    pool: &PgPool,
) -> Result<
    (
        String,
        Arc<dyn framework_file_manager::backend::StorageBackend>,
    ),
    String,
> {
    let settings = row.settings.as_ref().ok_or("settings 缺失")?;
    match provider {
        "local" => {
            // 配置级文件夹路径优先（stor-plc-path），回退 settings.base_path /
            // 固定默认路径（配置唯一真相源为 isahl 表，不读环境变量）
            let base_path = folder_path_for(pool, row.id)
                .await
                .or_else(|| {
                    settings
                        .get("base_path")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .unwrap_or_else(|| "./data/local-files".into());
            let backend = framework_file_manager::backend::LocalBackend::new(base_path, "/files");
            Ok(("local".into(), Arc::new(backend)))
        }
        _ => {
            let spec = provider_defaults(provider)
                .ok_or_else(|| format!("未知 storage provider: {provider}"))?;
            let enc = row.enc_fields.as_ref().ok_or("enc_fields 缺失")?;
            // aliyun_oss：access_key_id/access_key_secret → access_key/secret_key
            let access_key = enc
                .get("access_key")
                .or_else(|| enc.get("access_key_id"))
                .and_then(|v| v.as_str())
                .ok_or("access_key 缺失")?
                .to_string();
            let secret_key = enc
                .get("secret_key")
                .or_else(|| enc.get("access_key_secret"))
                .and_then(|v| v.as_str())
                .ok_or("secret_key 缺失")?
                .to_string();
            // 敏感凭证 AES-256-GCM 加密（enc: 前缀），解密后构造后端
            let access_key = decrypt_credential(&access_key);
            let secret_key = decrypt_credential(&secret_key);
            let region = settings
                .get("region")
                .and_then(|v| v.as_str())
                .unwrap_or(spec.default_region)
                .to_string();
            let endpoint = resolve_endpoint(&spec, settings)?;
            // settings.force_path_style 显式覆盖 provider 默认（minio 默认 true）
            let force_path_style = settings
                .get("force_path_style")
                .and_then(|v| v.as_bool())
                .unwrap_or(spec.default_path_style);
            let s3_cfg = framework_file_manager::backend::S3Config {
                endpoint,
                region,
                bucket: settings
                    .get("bucket")
                    .and_then(|v| v.as_str())
                    .ok_or("bucket 缺失")?
                    .to_string(),
                access_key,
                secret_key,
                cdn_domain: settings
                    .get("cdn_domain")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                force_path_style,
            };
            let backend = framework_file_manager::backend::S3Backend::new(&s3_cfg)
                .await
                .map_err(|e| format!("S3Backend 构造失败: {e}"))?;
            Ok((spec.scheme.into(), Arc::new(backend)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_defaults_covers_cloud_providers() {
        // 5 种 provider 全覆盖：scheme/默认 region/path-style/endpoint 模板
        let cases: &[(&str, &str, &str, bool, Option<&str>)] = &[
            ("s3", "s3", "us-east-1", false, None),
            (
                "aliyun_oss",
                "oss",
                "oss-cn-hangzhou",
                false,
                Some("{region}.aliyuncs.com"),
            ),
            (
                "tencent_cos",
                "cos",
                "ap-guangzhou",
                false,
                Some("cos.{region}.myqcloud.com"),
            ),
            (
                "huawei_obs",
                "obs",
                "cn-north-4",
                false,
                Some("obs.{region}.myhuaweicloud.com"),
            ),
            ("minio", "minio", "us-east-1", true, None),
        ];
        for (provider, scheme, region, path_style, tpl) in cases {
            let spec = provider_defaults(provider).unwrap_or_else(|| panic!("{provider} 应有默认"));
            assert_eq!(spec.scheme, *scheme, "{provider} scheme");
            assert_eq!(spec.default_region, *region, "{provider} region");
            assert_eq!(
                spec.default_path_style, *path_style,
                "{provider} path_style"
            );
            assert_eq!(spec.endpoint_template, *tpl, "{provider} endpoint 模板");
        }
        // 未知 provider → None（诚实失败）
        assert!(provider_defaults("unknown-x").is_none());
        assert!(provider_defaults("ceph").is_none());
    }

    #[test]
    fn resolve_endpoint_prefers_explicit_and_derives_template() {
        // 显式 endpoint 优先
        let settings = json!({"endpoint": "https://s3.example.com"});
        let spec = provider_defaults("s3").unwrap();
        assert_eq!(
            resolve_endpoint(&spec, &settings).unwrap(),
            "https://s3.example.com"
        );
        // 模板推导：region 显式覆盖
        let settings = json!({"region": "ap-shanghai"});
        let spec = provider_defaults("tencent_cos").unwrap();
        assert_eq!(
            resolve_endpoint(&spec, &settings).unwrap(),
            "cos.ap-shanghai.myqcloud.com"
        );
        // 模板推导：缺省 region → provider 默认
        let spec = provider_defaults("huawei_obs").unwrap();
        assert_eq!(
            resolve_endpoint(&spec, &json!({})).unwrap(),
            "obs.cn-north-4.myhuaweicloud.com"
        );
        // 无模板 provider（s3/minio）缺 endpoint → 诚实报错
        let spec = provider_defaults("s3").unwrap();
        assert!(resolve_endpoint(&spec, &json!({})).is_err());
        let spec = provider_defaults("minio").unwrap();
        assert!(resolve_endpoint(&spec, &json!({})).is_err());
        // 空字符串 endpoint 视为未配置（走模板/报错）
        let spec = provider_defaults("aliyun_oss").unwrap();
        assert_eq!(
            resolve_endpoint(&spec, &json!({"endpoint": ""})).unwrap(),
            "oss-cn-hangzhou.aliyuncs.com"
        );
    }
}
