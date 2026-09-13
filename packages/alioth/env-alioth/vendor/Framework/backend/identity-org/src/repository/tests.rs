// tests（split 自 repository.rs 内嵌模块，④ 候选）
use super::normalize_circle_point;
use serde_json::json;

/// 圆心输入规范化：GeoJSON Point / {lng,lat} / {lat,lng}
#[test]
fn test_normalize_circle_point() {
    // GeoJSON Point
    let g = json!({"type":"Point","coordinates":[113.752,23.021]});
    assert_eq!(normalize_circle_point(&g).unwrap(), (113.752, 23.021));
    // {lng,lat}
    assert_eq!(
        normalize_circle_point(&json!({"lng": 114.057, "lat": 22.543})).unwrap(),
        (114.057, 22.543)
    );
    // {lat,lng}（兼容旧形态）
    assert_eq!(
        normalize_circle_point(&json!({"lat": 22.543, "lng": 114.057})).unwrap(),
        (114.057, 22.543)
    );
    // 非法：缺坐标
    assert!(normalize_circle_point(&json!({"type": "Point"})).is_err());
    // 非法：非 Point 类型
    assert!(normalize_circle_point(&json!({"type": "Polygon", "coordinates": [[0,0]]})).is_err());
    // 非法：缺 lng
    assert!(normalize_circle_point(&json!({"lat": 1.0})).is_err());
}
