use std::{collections::BTreeMap, fs, io::ErrorKind, path::Path};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize, Serialize)]
pub struct Venue {
    #[serde(default = "default_location_template")]
    pub(crate) location_template: String,
    buildings: BTreeMap<String, Building>,
    events: BTreeMap<String, VenueEvent>,
}

fn default_location_template() -> String {
    "場所は{場所}です。".into()
}

#[derive(Debug, Deserialize, Serialize)]
struct Building {
    name: String,
    pin: MapPin,
    #[serde(default)]
    directions: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct MapPin {
    x: f64,
    y: f64,
}

#[derive(Debug, Deserialize, Serialize)]
struct VenueEvent {
    name: String,
    description: String,
    building: String,
    #[serde(default)]
    floor: String,
    #[serde(default)]
    room: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Destination {
    pub label: String,
    pub pin: MapPin,
}

impl Venue {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let bytes = match fs::read(path) {
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            result => result.with_context(|| format!("{} を読み込めません", path.display()))?,
        };
        let venue: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("{} の会場案内データが不正です", path.display()))?;
        venue.validate()?;
        Ok(Some(venue))
    }

    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.location_template.contains("{場所}"),
            "場所を伝える一文に {} を入れてください",
            "{場所}"
        );
        for (id, building) in &self.buildings {
            ensure!(
                !id.trim().is_empty()
                    && !building.name.trim().is_empty()
                    && [building.pin.x, building.pin.y]
                        .iter()
                        .all(|v| (0.0..=100.0).contains(v)),
                "venue.json の棟 {id} の名称またはピン座標が不正です"
            );
        }
        for (id, event) in &self.events {
            ensure!(
                self.buildings.contains_key(&event.building)
                    && [id, &event.name, &event.description]
                        .iter()
                        .all(|v| !v.trim().is_empty()),
                "venue.json の企画 {id} に未登録の棟または空の項目があります"
            );
        }
        Ok(())
    }

    pub fn catalog(&self) -> Map<String, Value> {
        self.events
            .iter()
            .map(|(id, event)| {
                let building = &self.buildings[&event.building];
                let entry = json!({
                    "name": event.name, "description": event.description,
                    "building": building.name, "floor": event.floor.trim(), "room": event.room.trim(),
                    "directions": building.directions.trim()
                });
                (id.clone(), entry)
            })
            .collect()
    }

    pub fn guide(&self, id: &str) -> Option<Destination> {
        let event = self.events.get(id)?;
        let building = &self.buildings[&event.building];
        Some(Destination {
            label: [building.name.trim(), event.floor.trim(), event.room.trim()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            pin: building.pin,
        })
    }
}
