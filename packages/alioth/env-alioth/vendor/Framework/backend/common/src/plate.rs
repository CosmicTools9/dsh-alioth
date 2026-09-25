//! 号牌（车牌）规则 —— **全平台唯一实现**（服务端为准）。
//!
//! 口径来源（用户裁决 2026-09-18「新建车牌不能重复」「要符合国家车牌的规则」；
//! 载体口径见 change `align-vehicle-plate-identity` 2026-09-23）：
//! - 号牌号 MUST 存 `zc_id_identity.identity`（分类字典 `zc_id_cate-identity.code = 'plate'`，
//!   经 `zc_id_entity_rr_identity` 关联车辆）；MUST NOT 存车辆表的 `notice`/`code`。
//! - 入库值 = [`normalize_plate`] 归一值（去空白与分隔圆点 + ASCII 大写），
//!   形态由 [`plate_format_ok`] 校验（GA 36-2018 民用号牌）。
//!
//! 消费方：门户登记 `OpenActivity/backend/src/handlers/portal_write.rs`、
//! 平台号牌写径 `identity-org`（`plates.rs` / `handlers/vehicle_plates.rs`）。
//! 前端即时反馈镜像：`OpenActivity/frontend/src/lib/plate.ts`、
//! `Pre-Proc/WZ/Sources/Apps/Modules/fleet-wz/frontend/src/lib/plate.ts`（判据与本文逐条一致）。

/// 省级简称词表（31 个；GA 36-2018 民用号牌第一位）。
/// 不含 军/空/武警 等特种号牌首字（本平台登记的是公路营运车队，见 [`plate_format_ok`] 覆盖说明）。
pub const PLATE_PROVINCES: &str = "京津冀晋蒙辽吉黑沪苏浙皖闽赣鲁豫鄂湘粤桂琼渝川贵云藏陕甘青宁新";

/// 号牌字母位（发牌机关代号与序号字母同词表）：A-Z 去 I/O（GA 36-2018 规避与 1/0 混淆）。
pub fn is_plate_letter(c: char) -> bool {
    c.is_ascii_uppercase() && c != 'I' && c != 'O'
}

/// 序号位字符：阿拉伯数字或号牌字母。
pub fn is_plate_serial(c: char) -> bool {
    c.is_ascii_digit() || is_plate_letter(c)
}

/// 号牌归一（**入口唯一实现**）：去空白与分隔圆点 + ASCII 字母大写。
///
/// 目的 = 唯一性判定不被写法差异绕过（`蒙 b d22345` / `京A·12345` ≡ `蒙BD22345` / `京A12345`）；
/// 归一后的值即落库值（`zc_id_identity.identity`），故历史行只可能比新写法更「原始」，
/// 判重侧再以 `upper()` 兜一层（旧行小写亦命中）。
pub fn normalize_plate(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '.' | '·' | '•'))
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// 中国民用号牌形态校验（**本项目唯一实现**——平台号牌写径与门户登记共用；
/// 前端即时反馈镜像见 `OpenActivity/frontend/src/lib/plate.ts`，服务端为准）。
///
/// 规则依据 GA 36-2018《中华人民共和国机动车号牌》（民用号牌部分）：
/// - 标准号牌（7 位）：省简称 + 发牌机关字母 + 5 位序号（字母/数字混合，无 I/O）；
/// - 新能源号牌（8 位）：小型车 = 省简称 + 发牌机关字母 + `D`/`F` + 5 位数字；
///   大型车 = 省简称 + 发牌机关字母 + 5 位数字 + `D`/`F`；
/// - 民用特型（7 位）：省简称 + 发牌机关字母 + 4 位序号 + `挂`（挂车）/ `学`（教练车）。
///
/// **覆盖范围与排除**：覆盖上述公路营运常见民用形态（含新能源两类）；
/// **不覆盖**军车（军/空/武警白底）、临时行驶车号牌、使领馆（使/领）、港澳入出境（粤Z*港/澳）、
/// 民航/农用等非公路号牌，以及应急/试验等细分形态——本平台采集的是营运车队，
/// 上述形态既无对应业务场景，其序号规则也不在民用简称词表内（会被拒绝）。
pub fn plate_format_ok(plate: &str) -> bool {
    let cs: Vec<char> = plate.chars().collect();
    if !(7..=8).contains(&cs.len()) {
        return false;
    }
    if !PLATE_PROVINCES.contains(cs[0]) || !is_plate_letter(cs[1]) {
        return false;
    }
    let tail = &cs[2..];
    match tail.len() {
        // 标准 5 位序号；或 4 位序号 + 民用特型后缀（挂 / 学）
        5 => {
            tail.iter().all(|&c| is_plate_serial(c))
                || (tail[..4].iter().all(|&c| is_plate_serial(c)) && matches!(tail[4], '挂' | '学'))
        }
        // 新能源 6 位序号（小型 D/F 在前，大型 D/F 在后）
        6 => {
            (matches!(tail[0], 'D' | 'F') && tail[1..].iter().all(char::is_ascii_digit))
                || (tail[..5].iter().all(char::is_ascii_digit) && matches!(tail[5], 'D' | 'F'))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_separators_and_uppercases() {
        assert_eq!(normalize_plate(" 京a·12345 "), "京A12345");
        assert_eq!(normalize_plate("蒙 b d22345"), "蒙BD22345");
    }

    #[test]
    fn format_accepts_standard_new_energy_and_trailer() {
        for ok in [
            "京A12345",
            "蒙BD22345",
            "沪AD12345",
            "京A12345D",
            "京A1234挂",
            "京A1234学",
        ] {
            assert!(plate_format_ok(ok), "{ok} 应通过");
        }
    }

    #[test]
    fn format_rejects_special_and_malformed() {
        for bad in [
            "",
            "京A1234",    // 位数不足
            "京A123456",  // 位数超出
            "使A12345",   // 使领馆不在民用简称词表
            "京I12345",   // I/O 位禁用
            "京AO1234",   // 发牌机关位禁用 O
            "蒙b d22345", // 未归一（小写/空白）
        ] {
            assert!(!plate_format_ok(bad), "{bad} 应被拒");
        }
    }
}
