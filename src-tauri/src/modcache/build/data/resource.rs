//! 资源类型（金币 / 木材 / 水银 / 矿石 / 硫磺 / 水晶 / 宝石 / 秘银）。
//!
//! 数据来源：
//! - `config/resources.json`（游戏根）：`id → {index, price}`；
//! - 名称：`resource.<scope>.<id>.name`（mod 翻译）→ `resource.core.<id>.name`（h3-core，中文=木材/水银/…）
//!   → 配置内嵌 `name` → id。VCMI 翻译池与 config **均无中文名**（实测 0 条），故由官方译名烘入 h3-core
//!   （见 `scripts/gen-h3-core-texts.mjs` 的 `RESOURCE_NAMES`）；
//! - 图标：`core/images/resources/<index>.webp`（由 `global::extract_resource_icons_for_mod` 生成，
//!   阶段二产出；阶段一 image 为 null）。

use std::path::Path;

use serde_json::{json, Value};

use crate::infra::json::parse_vcmi_json;
use crate::modcache::i18n::{i18n_mod_scope, name_obj_multi_key};
use crate::vcmi::h3core::I18nMap;

/// 构建资源列表（投影给启动器：宝物价格、建筑花费、资源图标提示等场景可按 id/index 取用）。
pub(crate) fn build_resources(
    game_dir: &Path,
    i18n: &I18nMap,
    mod_id: &str,
) -> Vec<Value> {
    let mut out = Vec::new();
    let path = game_dir.join("config").join("resources.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return out;
    };
    let Value::Object(m) = parse_vcmi_json(&text) else {
        return out;
    };
    let scope = i18n_mod_scope(mod_id);
    let with_img = crate::modcache::assets_enabled();
    for (id, obj) in m {
        if !obj.is_object() {
            continue;
        }
        let index = obj.get("index").and_then(|v| v.as_i64()).unwrap_or(-1);
        let price = obj.get("price").and_then(|v| v.as_i64()).unwrap_or(0);
        let name = name_obj_multi_key(
            i18n,
            &[
                format!("resource.{}.{}.name", scope, id),
                format!("resource.core.{}.name", id),
            ],
            obj.get("name"),
            &id,
        );
        out.push(json!({
            "id": id,
            "index": index,
            "price": price,
            "name": name,
            // 图标由 core 伪模组产出（跨 mod 共用），阶段二才有文件
            "image": if with_img && index >= 0 {
                json!(format!("core/images/resources/{}.webp", index))
            } else {
                Value::Null
            },
        }));
    }
    out.sort_by_key(|v| v.get("index").and_then(|x| x.as_i64()).unwrap_or(i64::MAX));
    out
}
