//! Config Schema Definitions
//!
//! 预定义各类外部服务接入的配置 Schema，用于驱动前端动态表单生成。
//! Schema 与业务解耦，新增分类只需在此注册即可。

use crate::models::{ConfigCategory, ConfigFieldSchema, ConfigProvider, SelectOption};

/// 获取所有支持的配置分类及 Schema
pub fn get_all_categories() -> Vec<ConfigCategory> {
    vec![
        llm_category(),
        email_category(),
        im_category(),
        webhook_category(),
        storage_category(),
        sms_category(),
    ]
}

/// 根据分类 code 获取 Schema
pub fn get_category(code: &str) -> Option<ConfigCategory> {
    get_all_categories().into_iter().find(|c| c.code == code)
}

/// 根据分类 code 和提供商 code 获取字段列表
pub fn get_provider_schema(
    category_code: &str,
    provider_code: &str,
) -> Option<Vec<ConfigFieldSchema>> {
    get_category(category_code)?
        .providers
        .into_iter()
        .find(|p| p.code == provider_code)
        .map(|p| p.schema)
}

/// 获取指定分类下所有敏感字段 key
pub fn get_sensitive_keys(category_code: &str, provider_code: &str) -> Vec<String> {
    get_provider_schema(category_code, provider_code)
        .map(|schema| {
            schema
                .into_iter()
                .filter(|f| f.sensitive)
                .map(|f| f.key)
                .collect()
        })
        .unwrap_or_default()
}

// ============================================
// LLM
// ============================================

/// LLM 供应商预设（base_url/model/flash_model 与 `Framework/backend/llm` 各 backend
/// 的 DEFAULT_* 常量保持一致——system-config 与 llm 解耦（schema 件不感知业务 crate），
/// 一致性由 `scripts/check/check-llm-presets-alignment.sh` 门禁比对，禁止单侧改动。
fn llm_preset(base_url: &str, model: &str, flash_model: &str) -> serde_json::Value {
    serde_json::json!({
        "base_url": base_url,
        "model": model,
        "flash_model": flash_model,
    })
}

fn llm_provider(
    code: &str,
    notice: &str,
    description: &str,
    defaults: Option<serde_json::Value>,
) -> ConfigProvider {
    ConfigProvider {
        code: code.to_string(),
        notice: notice.to_string(),
        description: Some(description.to_string()),
        schema: llm_common_schema(),
        defaults,
    }
}

fn llm_category() -> ConfigCategory {
    ConfigCategory {
        code: "llm".to_string(),
        notice: "大语言模型".to_string(),
        description: Some("配置 LLM 服务提供商接入".to_string()),
        icon: Some("Brain".to_string()),
        providers: vec![
            llm_provider(
                "deepseek",
                "DeepSeek",
                "DeepSeek API 接入",
                Some(llm_preset(
                    "https://api.deepseek.com",
                    "deepseek-v4-pro",
                    "deepseek-v4-flash",
                )),
            ),
            llm_provider(
                "kimi",
                "Kimi (月之暗面)",
                "Moonshot Kimi API 接入",
                Some(llm_preset(
                    "https://api.kimi.com/coding",
                    "k3-256k",
                    "kimi-for-coding",
                )),
            ),
            llm_provider(
                "glm",
                "GLM (智谱)",
                "智谱 GLM Coding Plan / 开放平台 API 接入",
                Some(llm_preset(
                    "https://open.bigmodel.cn/api/coding/paas/v4",
                    "glm-5.3",
                    "glm-5.3-flash",
                )),
            ),
            llm_provider(
                "minimax",
                "MiniMax",
                "MiniMax 开放平台 API 接入",
                Some(llm_preset(
                    "https://api.minimaxi.com",
                    "MiniMax-M3",
                    "MiniMax-M2.7",
                )),
            ),
            // OpenAI 官方端点（OpenAI 兼容协议，后端经通用兼容层接入）
            llm_provider(
                "openai",
                "OpenAI",
                "OpenAI API 接入",
                Some(serde_json::json!({
                    "base_url": "https://api.openai.com/v1",
                    "model": "gpt-4o",
                    "flash_model": "gpt-4o-mini",
                })),
            ),
            // Anthropic Messages API 与 OpenAI 兼容层协议不同，不给预设（防静默配错）
            llm_provider("anthropic", "Anthropic", "Claude API 接入", None),
            // 自定义兼容服务：地址/模型由用户自定，无预设
            llm_provider("custom", "自定义兼容", "兼容 OpenAI 协议的自定义服务", None),
        ],
    }
}

fn llm_common_schema() -> Vec<ConfigFieldSchema> {
    vec![
        with_help(
            with_placeholder(
                field("base_url", "API 地址", "url", false),
                "https://api.openai.com/v1",
            ),
            "服务商 API 基础地址",
        ),
        with_help(
            with_placeholder(field("api_key", "API 密钥", "password", true), "sk-..."),
            "服务商提供的 API Key",
        ),
        with_help(
            with_placeholder(field("model", "默认模型", "text", false), "gpt-4o"),
            "默认使用的模型名称",
        ),
        with_help(
            with_placeholder(
                field("flash_model", "快速模型", "text", false),
                "gpt-4o-mini",
            ),
            "轻量级快速模型名称，用于非推理类任务",
        ),
        with_help(
            with_default(
                field("timeout", "超时时间(秒)", "number", false),
                serde_json::json!(120),
            ),
            "请求超时时间",
        ),
        with_help(
            with_default(
                field("max_retries", "最大重试次数", "number", false),
                serde_json::json!(2),
            ),
            "请求失败时的最大重试次数",
        ),
        with_help(
            with_default(
                field("temperature", "Temperature", "number", false),
                serde_json::json!(0.7),
            ),
            "生成随机性参数 (0-2)",
        ),
    ]
}

// ============================================
// Email
// ============================================

fn email_category() -> ConfigCategory {
    ConfigCategory {
        code: "email".to_string(),
        notice: "邮件服务".to_string(),
        description: Some("配置 SMTP 邮件发送服务".to_string()),
        icon: Some("Mail".to_string()),
        providers: vec![ConfigProvider {
            code: "smtp".to_string(),
            notice: "SMTP".to_string(),
            description: Some("通用 SMTP 协议".to_string()),
            schema: vec![
                with_help(
                    with_placeholder(
                        field("host", "服务器地址", "text", true),
                        "smtp.example.com",
                    ),
                    "SMTP 服务器主机名",
                ),
                with_help(
                    with_default(
                        field("port", "端口", "number", true),
                        serde_json::json!(587),
                    ),
                    "SMTP 端口，通常为 25/587/465",
                ),
                with_help(
                    with_placeholder(
                        field("username", "用户名", "text", true),
                        "noreply@example.com",
                    ),
                    "SMTP 登录用户名",
                ),
                with_help(
                    with_placeholder(
                        field("password", "密码/授权码", "password", true),
                        "********",
                    ),
                    "SMTP 密码或授权码",
                ),
                with_help(
                    with_placeholder(
                        field("from_address", "发件人地址", "text", true),
                        "noreply@example.com",
                    ),
                    "邮件显示的发件人地址",
                ),
                with_help(
                    with_placeholder(
                        field("from_name", "发件人名称", "text", false),
                        "AliothStudio",
                    ),
                    "邮件显示的发件人名称",
                ),
                with_help(
                    with_default(
                        field("use_tls", "启用 TLS", "boolean", false),
                        serde_json::json!(true),
                    ),
                    "是否使用 TLS 加密连接",
                ),
            ],
            defaults: None,
        }],
    }
}

// ============================================
// IM (Instant Messaging)
// ============================================

fn im_category() -> ConfigCategory {
    ConfigCategory {
        code: "im".to_string(),
        notice: "即时通讯".to_string(),
        description: Some("配置企业微信、钉钉、飞书等 IM 接入".to_string()),
        icon: Some("MessageSquare".to_string()),
        providers: vec![
            ConfigProvider {
                code: "wecom".to_string(),
                notice: "企业微信".to_string(),
                description: Some("企业微信应用机器人接入".to_string()),
                schema: vec![
                    with_help(
                        with_placeholder(
                            field("corp_id", "企业 ID", "text", true),
                            "wwxxxxxxxxxxxxxxxx",
                        ),
                        "企业微信 CorpID",
                    ),
                    with_help(
                        with_placeholder(field("agent_id", "应用 ID", "text", true), "1000002"),
                        "企业微信应用 AgentId",
                    ),
                    with_help(
                        with_placeholder(field("secret", "应用密钥", "password", true), "********"),
                        "企业微信应用 Secret",
                    ),
                    with_help(
                        with_placeholder(
                            field("webhook_url", "Webhook 地址", "url", false),
                            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=...",
                        ),
                        "群机器人 Webhook 地址（可选）",
                    ),
                ],
                defaults: None,
            },
            ConfigProvider {
                code: "dingtalk".to_string(),
                notice: "钉钉".to_string(),
                description: Some("钉钉群机器人/应用接入".to_string()),
                schema: vec![
                    with_help(
                        with_placeholder(
                            field("app_key", "AppKey", "text", true),
                            "dingxxxxxxxxxxxxxxxx",
                        ),
                        "钉钉应用 AppKey",
                    ),
                    with_help(
                        with_placeholder(
                            field("app_secret", "AppSecret", "password", true),
                            "********",
                        ),
                        "钉钉应用 AppSecret",
                    ),
                    with_help(
                        with_placeholder(
                            field("webhook_url", "Webhook 地址", "url", false),
                            "https://oapi.dingtalk.com/robot/send?access_token=...",
                        ),
                        "群机器人 Webhook 地址（可选）",
                    ),
                    with_help(
                        with_placeholder(
                            field("webhook_secret", "Webhook 密钥", "password", false),
                            "SECxxxxxxxxxxxxxxxx",
                        ),
                        "群机器人加签密钥（可选）",
                    ),
                ],
                defaults: None,
            },
            ConfigProvider {
                code: "feishu".to_string(),
                notice: "飞书".to_string(),
                description: Some("飞书应用/群机器人接入".to_string()),
                schema: vec![
                    with_help(
                        with_placeholder(
                            field("app_id", "App ID", "text", true),
                            "cli_xxxxxxxxxxxxxxxx",
                        ),
                        "飞书应用 App ID",
                    ),
                    with_help(
                        with_placeholder(
                            field("app_secret", "App Secret", "password", true),
                            "********",
                        ),
                        "飞书应用 App Secret",
                    ),
                    with_help(
                        with_placeholder(
                            field("webhook_url", "Webhook 地址", "url", false),
                            "https://open.feishu.cn/open-apis/bot/v2/hook/...",
                        ),
                        "群机器人 Webhook 地址（可选）",
                    ),
                ],
                defaults: None,
            },
        ],
    }
}

// ============================================
// Webhook
// ============================================

fn webhook_category() -> ConfigCategory {
    ConfigCategory {
        code: "webhook".to_string(),
        notice: "Webhook".to_string(),
        description: Some("配置通用 Webhook 回调".to_string()),
        icon: Some("Webhook".to_string()),
        providers: vec![ConfigProvider {
            code: "generic".to_string(),
            notice: "通用 Webhook".to_string(),
            description: Some("支持自定义签名的通用 Webhook".to_string()),
            schema: vec![
                with_help(
                    with_placeholder(
                        field("url", "回调地址", "url", true),
                        "https://example.com/webhook",
                    ),
                    "Webhook 回调 URL",
                ),
                with_help(
                    with_default(
                        with_options(
                            field("method", "请求方法", "select", true),
                            vec![("POST", "POST"), ("PUT", "PUT"), ("PATCH", "PATCH")],
                        ),
                        serde_json::json!("POST"),
                    ),
                    "HTTP 请求方法",
                ),
                with_help(
                    with_placeholder(field("secret", "签名密钥", "password", false), "********"),
                    "用于 HMAC 签名的密钥（可选）",
                ),
                with_help(
                    with_placeholder(
                        field("headers", "自定义 Header", "textarea", false),
                        "Content-Type: application/json\nX-Custom: value",
                    ),
                    "每行一个 Header，格式为 Key: Value",
                ),
            ],
            defaults: None,
        }],
    }
}

// ============================================
// Storage
// ============================================

fn storage_category() -> ConfigCategory {
    ConfigCategory {
        code: "storage".to_string(),
        notice: "对象存储".to_string(),
        description: Some("配置本地磁盘 / OSS / S3 等文件存储接入".to_string()),
        icon: Some("HardDrive".to_string()),
        providers: vec![
            ConfigProvider {
                code: "local".to_string(),
                notice: "本地文件存储".to_string(),
                description: Some("本地磁盘存储（开发/单机部署）".to_string()),
                schema: vec![with_help(
                    with_placeholder(
                        field("base_path", "存储根目录", "text", false),
                        "./data/local-files",
                    ),
                    "本地文件存储根目录（相对 Gateway 工作目录或绝对路径）",
                )],
                defaults: None,
            },
            ConfigProvider {
                code: "s3".to_string(),
                notice: "S3 兼容".to_string(),
                description: Some("兼容 S3 协议的对象存储".to_string()),
                schema: vec![
                    with_help(
                        with_placeholder(
                            field("endpoint", "Endpoint", "url", true),
                            "https://s3.amazonaws.com",
                        ),
                        "S3 Endpoint 地址",
                    ),
                    with_help(
                        with_placeholder(field("region", "Region", "text", true), "us-east-1"),
                        "S3 Region",
                    ),
                    with_help(
                        with_placeholder(field("bucket", "Bucket", "text", true), "my-bucket"),
                        "存储桶名称",
                    ),
                    with_help(
                        with_placeholder(
                            field("access_key", "Access Key", "text", true),
                            "AKIA...",
                        ),
                        "访问密钥 ID",
                    ),
                    with_help(
                        with_placeholder(
                            field("secret_key", "Secret Key", "password", true),
                            "********",
                        ),
                        "访问密钥密码",
                    ),
                    with_help(
                        with_placeholder(
                            field("cdn_domain", "CDN 域名", "url", false),
                            "https://cdn.example.com",
                        ),
                        "文件访问 CDN 域名（可选）",
                    ),
                ],
                defaults: None,
            },
            ConfigProvider {
                code: "aliyun_oss".to_string(),
                notice: "阿里云 OSS".to_string(),
                description: Some("阿里云对象存储".to_string()),
                schema: vec![
                    with_help(
                        with_placeholder(
                            field("endpoint", "Endpoint", "url", true),
                            "https://oss-cn-hangzhou.aliyuncs.com",
                        ),
                        "OSS Endpoint",
                    ),
                    with_help(
                        with_placeholder(field("bucket", "Bucket", "text", true), "my-bucket"),
                        "存储桶名称",
                    ),
                    with_help(
                        with_placeholder(
                            field("access_key_id", "AccessKeyId", "text", true),
                            "LTAI...",
                        ),
                        "阿里云 AccessKeyId",
                    ),
                    with_help(
                        with_placeholder(
                            field("access_key_secret", "AccessKeySecret", "password", true),
                            "********",
                        ),
                        "阿里云 AccessKeySecret",
                    ),
                    with_help(
                        with_placeholder(
                            field("cdn_domain", "CDN 域名", "url", false),
                            "https://cdn.example.com",
                        ),
                        "文件访问 CDN 域名（可选）",
                    ),
                ],
                defaults: None,
            },
        ],
    }
}

// ============================================
// SMS
// ============================================

fn sms_category() -> ConfigCategory {
    ConfigCategory {
        code: "sms".to_string(),
        notice: "短信服务".to_string(),
        description: Some("配置短信发送服务商".to_string()),
        icon: Some("Smartphone".to_string()),
        providers: vec![ConfigProvider {
            code: "aliyun".to_string(),
            notice: "阿里云短信".to_string(),
            description: Some("阿里云短信服务".to_string()),
            schema: vec![
                with_help(
                    with_placeholder(
                        field("access_key_id", "AccessKeyId", "text", true),
                        "LTAI...",
                    ),
                    "阿里云 AccessKeyId",
                ),
                with_help(
                    with_placeholder(
                        field("access_key_secret", "AccessKeySecret", "password", true),
                        "********",
                    ),
                    "阿里云 AccessKeySecret",
                ),
                with_help(
                    with_placeholder(
                        field("sign_name", "短信签名", "text", true),
                        "阿里云短信测试",
                    ),
                    "已通过审核的短信签名",
                ),
                with_help(
                    with_placeholder(
                        field("template_code", "默认模板 CODE", "text", false),
                        "SMS_12345678",
                    ),
                    "默认短信模板 CODE",
                ),
            ],
            defaults: None,
        }],
    }
}

// ============================================
// Schema Builder Helpers
// ============================================

fn field(key: &str, notice: &str, field_type: &str, sensitive: bool) -> ConfigFieldSchema {
    ConfigFieldSchema {
        key: key.to_string(),
        notice: notice.to_string(),
        field_type: field_type.to_string(),
        required: true,
        placeholder: None,
        default_value: None,
        options: None,
        sensitive,
        help_text: None,
    }
}

fn with_placeholder(mut f: ConfigFieldSchema, placeholder: &str) -> ConfigFieldSchema {
    f.placeholder = Some(placeholder.to_string());
    f
}

fn with_default(mut f: ConfigFieldSchema, value: serde_json::Value) -> ConfigFieldSchema {
    f.default_value = Some(value);
    f
}

fn with_options(mut f: ConfigFieldSchema, opts: Vec<(&str, &str)>) -> ConfigFieldSchema {
    f.options = Some(
        opts.into_iter()
            .map(|(v, n)| SelectOption {
                value: v.to_string(),
                notice: n.to_string(),
            })
            .collect(),
    );
    f
}

fn with_help(mut f: ConfigFieldSchema, text: &str) -> ConfigFieldSchema {
    f.help_text = Some(text.to_string());
    f
}
