//! Map markers: the state `createMarker` and the `marker*` commands drive.
//!
//! A marker is a named entry in a table, not an Entity: the original keeps them in the World's
//! marker map, keyed by name, and the map UI draws them. Positions are world space (ADR 0003);
//! the script boundary swaps Y and Z as everywhere else.
//!
//! Values the engine returns verbatim (`markerType`, `markerColor`, `markerShape`,
//! `markerBrush`, `markerText`) are kept as the strings scripts set, with the shapes uppercased
//! the way `markerShape` reports them.

use std::collections::HashMap;

use glam::DVec3;

use crate::World;

/// The state of one map marker.
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    /// The name scripts use; unique in the World.
    pub name: String,
    /// World-space position (ADR 0003).
    pub position: DVec3,
    /// Rotation in degrees clockwise from north (`setMarkerDir`).
    pub direction: f32,
    /// `markerType`, e.g. `"hd_dot"`; empty until set, and an empty type draws nothing.
    pub marker_type: String,
    /// `markerShape`, uppercase: `"ICON"`, `"RECTANGLE"`, `"ELLIPSE"`, `"POLYLINE"`.
    pub shape: String,
    /// `markerSize`: `[a, b]` in metres (icons: a scale factor).
    pub size: [f32; 2],
    /// `markerColor`, e.g. `"ColorRed"` or a `#(r,g,b,a)` colour.
    pub color: String,
    /// `markerText`.
    pub text: String,
    /// `markerBrush`: `"Solid"`, `"Border"`, `"Horizontal"`, ... for area markers.
    pub brush: String,
    /// `markerAlpha`, 0..1.
    pub alpha: f32,
    /// `markerPolyline` points, world space (only meaningful for the polyline shape).
    pub polyline: Vec<DVec3>,
    /// `markerShadow`.
    pub shadow: bool,
    /// Draw order among markers: higher draws on top, and `allMapMarkers` sorts by it.
    pub draw_priority: f32,
    /// Multiplayer marker channel (`createMarker`'s third element), -1 when none.
    pub channel: i32,
    /// Created by `createMarkerLocal`: this machine only, never broadcast.
    pub local: bool,
}

impl Marker {
    /// A marker at `position` with the engine's defaults.
    pub fn new(name: impl Into<String>, position: DVec3) -> Self {
        Marker {
            name: name.into(),
            position,
            direction: 0.0,
            marker_type: String::new(),
            shape: "ICON".to_owned(),
            size: [1.0, 1.0],
            color: "Default".to_owned(),
            text: String::new(),
            brush: "Solid".to_owned(),
            alpha: 1.0,
            polyline: Vec::new(),
            shadow: true,
            draw_priority: 0.0,
            channel: -1,
            local: false,
        }
    }
}

/// The World's markers, in the order `allMapMarkers` reports them.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Markers {
    order: Vec<String>,
    by_name: HashMap<String, Marker>,
}

impl Markers {
    /// `createMarker`: inserts a marker. Fails (returning `false`) when the name is taken,
    /// which the engine ignores the command for.
    pub fn create(&mut self, marker: Marker) -> bool {
        if self.by_name.contains_key(&marker.name) {
            return false;
        }
        self.order.push(marker.name.clone());
        self.by_name.insert(marker.name.clone(), marker);
        true
    }

    /// `deleteMarker`. `false` when there was no such marker.
    pub fn delete(&mut self, name: &str) -> bool {
        if self.by_name.remove(name).is_none() {
            return false;
        }
        self.order.retain(|n| n != name);
        true
    }

    pub fn get(&self, name: &str) -> Option<&Marker> {
        self.by_name.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Marker> {
        self.by_name.get_mut(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Markers in `allMapMarkers` order: ascending draw priority, creation order breaking ties
    /// (the original sorts by priority; before 2.18 a new marker simply went last).
    pub fn iter(&self) -> impl Iterator<Item = &Marker> {
        let mut markers: Vec<&Marker> = self.order.iter().filter_map(|n| self.by_name.get(n)).collect();
        // `sort_by` is stable, so equal priorities keep creation order.
        markers.sort_by(|a, b| {
            a.draw_priority
                .partial_cmp(&b.draw_priority)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        markers.into_iter()
    }

    /// `setMarkerDrawPriority`: sets the priority; [`Markers::iter`] reflects the new order.
    pub fn set_draw_priority(&mut self, name: &str, priority: f32) {
        if let Some(m) = self.by_name.get_mut(name) {
            m.draw_priority = priority;
        }
    }
}

impl World {
    pub fn markers(&self) -> &Markers {
        &self.markers
    }

    pub fn markers_mut(&mut self) -> &mut Markers {
        &mut self.markers
    }

    /// `allMapMarkers`: the names, in draw order.
    pub fn marker_names(&self) -> Vec<&str> {
        self.markers.iter().map(|m| m.name.as_str()).collect()
    }
}
