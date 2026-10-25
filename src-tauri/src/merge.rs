//! Applying an update from the live feed to the state we hold. The feed first sends a
//! full snapshot, then small *deltas*. A delta is shaped like the data but holds only
//! what changed, and addresses array items by index: `{"Lines": {"3": {"Position": "1"}}}`
//! means "driver 3's position is now 1, leave the rest alone".
//! (Port of `merge` in the Electron app's `src/livetiming.js`.)

use serde_json::{Map, Value};

/// Keys the feed adds for its own bookkeeping.
const IGNORED_KEYS: [&str; 2] = ["_kf", "_deleted"];

/// Merges `patch` into `target` and returns the result. If `target` is missing, the patch
/// is built up from nothing.
pub fn merge(target: Option<Value>, patch: Value) -> Value {
    let mut out = target.unwrap_or(Value::Null);
    merge_into(&mut out, patch);
    out
}

/// Merges `patch` into `target` in place.
///
/// - An object patch is applied key by key, into an object or (with numeric keys) into
///   an array, growing it if the index is past the end.
/// - Anything else (text, a number, `null`, a whole array) simply replaces what was there.
pub fn merge_into(target: &mut Value, patch: Value) {
    let Value::Object(entries) = patch else {
        *target = patch;
        return;
    };
    // Only objects and arrays can be patched into; anything else is replaced by an object.
    if !target.is_object() && !target.is_array() {
        *target = Value::Object(Map::new());
    }
    for (key, value) in entries {
        if IGNORED_KEYS.contains(&key.as_str()) {
            continue;
        }
        match target {
            Value::Array(items) => {
                let Ok(index) = key.parse::<usize>() else {
                    continue;
                };
                if index >= items.len() {
                    items.resize(index + 1, Value::Null);
                }
                merge_into(&mut items[index], value);
            }
            Value::Object(map) => merge_into(map.entry(key).or_insert(Value::Null), value),
            _ => {} // not reachable: the target was made an object or array above
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patches_nested_objects_and_array_items_by_index() {
        let mut state = json!({"Lines": {"1": {"Sectors": [{"Value": "27.1"}, {"Value": "39.0"}], "Position": "1"}}});
        merge_into(
            &mut state,
            json!({"Lines": {"1": {"Sectors": {"1": {"Value": "38.9"}}}}}),
        );
        assert_eq!(
            state["Lines"]["1"]["Sectors"],
            json!([{"Value": "27.1"}, {"Value": "38.9"}])
        );
        assert_eq!(state["Lines"]["1"]["Position"], "1"); // untouched
    }

    #[test]
    fn ignores_the_feeds_bookkeeping_keys() {
        assert_eq!(
            merge(None, json!({"Status": "1", "_kf": true})),
            json!({"Status": "1"})
        );
        assert_eq!(
            merge(None, json!({"A": 1, "_deleted": ["x"]})),
            json!({"A": 1})
        );
    }

    #[test]
    fn values_and_whole_arrays_replace_what_was_there() {
        assert_eq!(
            merge(Some(json!({"A": 1})), json!({"A": "two"})),
            json!({"A": "two"})
        );
        assert_eq!(
            merge(Some(json!({"A": [1, 2, 3]})), json!({"A": [9]})),
            json!({"A": [9]})
        );
        assert_eq!(
            merge(Some(json!({"A": 1})), json!({"A": null})),
            json!({"A": null})
        );
    }

    #[test]
    fn an_index_past_the_end_grows_the_array() {
        let mut state = json!({"Stints": [{"Compound": "SOFT"}]});
        merge_into(&mut state, json!({"Stints": {"2": {"Compound": "HARD"}}}));
        assert_eq!(
            state["Stints"],
            json!([{"Compound": "SOFT"}, null, {"Compound": "HARD"}])
        );
    }

    #[test]
    fn a_missing_or_wrong_kind_of_target_becomes_an_object() {
        assert_eq!(merge(None, json!({"A": {"B": 1}})), json!({"A": {"B": 1}}));
        assert_eq!(merge(Some(json!("text")), json!({"A": 1})), json!({"A": 1}));
    }

    #[test]
    fn a_non_numeric_key_into_an_array_is_skipped() {
        let mut state = json!([1, 2]);
        merge_into(&mut state, json!({"x": 5, "1": 20}));
        assert_eq!(state, json!([1, 20]));
    }
}
