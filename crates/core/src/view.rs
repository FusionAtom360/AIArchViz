//! The view context (9.1 get_view_context, 9.3): where the user's camera is,
//! which way it looks, what is selected and what the pointer is over.
//!
//! The app fills it from its 3D camera (T-206); the agent tools read it to
//! resolve "left", "this" and "that". Everything is an integer: the eye in
//! nanometers, the heading in millidegrees with 0 = +X (east),
//! counterclockwise (Q-5), and the pitch in signed millidegrees.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::document::Millidegrees;
use crate::identity::Uuid;
use crate::length::Length;

/// A camera's tilt in thousandths of a degree: positive looks up, negative
/// down, in [-90000, 90000].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "i32", into = "i32")]
pub struct CameraPitch(i32);

impl CameraPitch {
    /// Straight up or straight down.
    pub const LIMIT: i32 = 90_000;
    /// Looking at the horizon.
    pub const LEVEL: CameraPitch = CameraPitch(0);

    /// `value` millidegrees, if it lies in [-90000, 90000].
    pub const fn new(value: i32) -> Option<Self> {
        if value >= -Self::LIMIT && value <= Self::LIMIT {
            Some(CameraPitch(value))
        } else {
            None
        }
    }

    pub const fn value(self) -> i32 {
        self.0
    }
}

impl TryFrom<i32> for CameraPitch {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, String> {
        CameraPitch::new(value)
            .ok_or_else(|| format!("{value} millidegrees is not in [-90000, 90000]"))
    }
}

impl From<CameraPitch> for i32 {
    fn from(p: CameraPitch) -> i32 {
        p.0
    }
}

impl JsonSchema for CameraPitch {
    fn schema_name() -> Cow<'static, str> {
        "CameraPitch".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "integer", "minimum": -90_000, "maximum": 90_000 })
    }
}

/// Where the camera is and which way it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CameraPose {
    /// The eye, in world coordinates (5.4).
    pub x: Length,
    pub y: Length,
    pub z: Length,
    /// The horizontal direction the camera faces.
    pub heading: Millidegrees,
    /// How far the camera looks above or below the horizon.
    pub pitch: CameraPitch,
}

/// What get_view_context returns (9.1): the camera pose, what is selected
/// and what the pointer is over, by entity id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewContext {
    pub camera: CameraPose,
    /// Selected entities, sorted.
    pub selection: Vec<Uuid>,
    pub hovered: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::to_canonical_json;

    fn context() -> ViewContext {
        ViewContext {
            camera: CameraPose {
                x: Length::from_nanometers(1_000),
                y: Length::from_nanometers(-2_000),
                z: Length::from_nanometers(1_600_200_000),
                heading: Millidegrees::new(90_000).unwrap_or_else(|| panic!()),
                pitch: CameraPitch::new(-15_000).unwrap_or_else(|| panic!()),
            },
            selection: vec![Uuid::from_bytes([
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
            ])],
            hovered: None,
        }
    }

    #[test]
    fn the_view_context_is_canonical_json_of_integers() {
        let json = to_canonical_json(&context()).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            json,
            "{\"camera\":{\"heading\":90000,\"pitch\":-15000,\"x\":1000,\"y\":-2000,\
             \"z\":1600200000},\"hovered\":null,\
             \"selection\":[\"00000000-0000-0000-0000-000000000001\"]}"
        );
        let back: ViewContext = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, context());
    }

    #[test]
    fn pitch_outside_straight_up_and_down_is_rejected() {
        assert_eq!(
            CameraPitch::new(90_000).map(CameraPitch::value),
            Some(90_000)
        );
        assert_eq!(
            CameraPitch::new(-90_000).map(CameraPitch::value),
            Some(-90_000)
        );
        assert!(CameraPitch::new(90_001).is_none() && CameraPitch::new(-90_001).is_none());
        let json = "{\"camera\":{\"heading\":0,\"pitch\":90001,\"x\":0,\"y\":0,\"z\":0},\
                    \"hovered\":null,\"selection\":[]}";
        assert!(serde_json::from_str::<ViewContext>(json).is_err());
        let json = json
            .replace("\"pitch\":90001", "\"pitch\":0")
            .replace("0}", "0,\"w\":0}");
        assert!(serde_json::from_str::<ViewContext>(&json).is_err());
    }
}
