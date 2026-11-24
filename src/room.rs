//! Salle en 3D sur l'iPad : la scène des LEDs (bandeaux, décor, appareils DMX, lasers) envoyée
//! à la tablette, déjà calculée (position de chaque LED, faces du décor), et ce que la tablette
//! y change : déplacer, tourner, ajouter, dupliquer, supprimer.
//!
//! Les objets sont désignés par une lettre et leur numéro : « f3 » (bandeau de LEDs), « s2 »
//! (décor), « d1 » (appareil DMX), « l1 » (laser).

use serde::Deserialize;
use serde_json::{Value, json};

use crate::leds::{Fixture, FixtureKind, LedSetup, Surface};

/// Message de l'onglet Salle 3D de l'iPad.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum RoomMsg {
    /// La page montre la salle (renvoyé régulièrement) : l'app lui envoie la scène.
    Watch,
    /// Nouvelle position (m) et, si donné, nouveau cap (degrés autour de la verticale).
    Place {
        id: String,
        pos: [f32; 3],
        #[serde(default)]
        yaw: Option<f32>,
    },
    /// Ajoute un objet : « fixture:Strip », « surface:Écran »… posé en `pos` (au sol) si donné.
    Add {
        what: String,
        #[serde(default)]
        pos: Option<[f32; 3]>,
    },
    Dup { id: String },
    Del { id: String },
}

/// Taille maximale de la salle (m) : un objet ne part pas à l'infini.
const FAR: f32 = 200.0;

fn round(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

fn r3(p: [f32; 3]) -> [f32; 3] {
    p.map(round)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn ok(p: [f32; 3]) -> Option<[f32; 3]> {
    p.iter().all(|v| v.is_finite()).then(|| p.map(|v| v.clamp(-FAR, FAR)))
}

/// La scène pour l'iPad (message "room").
pub fn room_json(setup: &LedSetup) -> String {
    let mut items: Vec<Value> = Vec::new();
    for f in &setup.fixtures {
        items.push(json!({
            "id": format!("f{}", f.id), "cat": "fixture", "kind": f.kind.name(), "name": f.name,
            "pos": r3(f.pos), "yaw": round(f.rot[0]),
            "leds": f.positions().into_iter().map(r3).collect::<Vec<_>>(),
        }));
    }
    for s in &setup.surfaces {
        let faces: Vec<Value> = s
            .faces(0.0)
            .iter()
            .map(|q| json!({ "q": q.quad.map(r3), "c": hex(q.color), "o": q.output }))
            .collect();
        items.push(json!({
            "id": format!("s{}", s.id), "cat": "surface", "kind": s.kind.name(), "name": s.name,
            "pos": r3(s.pos), "yaw": round(s.rot[0]), "faces": faces,
        }));
    }
    for d in &setup.dmx {
        items.push(json!({
            "id": format!("d{}", d.id), "cat": "dmx", "kind": "DMX", "name": d.name,
            "pos": r3(d.pos), "yaw": round(d.rot[0]), "pitch": round(d.rot[1]),
        }));
    }
    for l in &setup.lasers {
        items.push(json!({
            "id": format!("l{}", l.id), "cat": "laser", "kind": "Laser", "name": l.name,
            "pos": r3(l.pos), "yaw": round(l.rot[0]), "pitch": round(l.rot[1]),
        }));
    }
    let fixtures: Vec<Value> = FixtureKind::ALL
        .iter()
        .filter(|k| **k != FixtureKind::Path)
        .map(|k| json!([format!("fixture:{k:?}"), k.name()]))
        .collect();
    let surfaces: Vec<Value> = Surface::presets(setup).iter().map(|(n, _)| json!([format!("surface:{n}"), n])).collect();
    json!({ "t": "room", "room": setup.room, "items": items, "add": { "leds": fixtures, "decor": surfaces } }).to_string()
}

/// Applique un changement venu de l'iPad. Renvoie l'objet à choisir ensuite (ajouté, copié).
pub fn apply(setup: &mut LedSetup, msg: &RoomMsg) -> Option<String> {
    match msg {
        RoomMsg::Watch => None,
        RoomMsg::Place { id, pos, yaw } => {
            let pos = ok(*pos)?;
            let yaw = yaw.filter(|y| y.is_finite()).map(|y| y.rem_euclid(360.0));
            let (pos_mut, rot_mut) = find(setup, id)?;
            *pos_mut = pos;
            if let Some(y) = yaw {
                rot_mut[0] = y;
            }
            None
        }
        RoomMsg::Add { what, pos } => {
            let at = pos.and_then(ok);
            let (cat, name) = what.split_once(':')?;
            match cat {
                "fixture" => {
                    let kind = FixtureKind::ALL.into_iter().find(|k| format!("{k:?}") == name)?;
                    let id = setup.next_fixture_id();
                    let n = setup.fixtures.iter().filter(|f| f.kind == kind).count() + 1;
                    let mut f = Fixture::new(kind, id, format!("{} {n}", kind.name()));
                    // Branché comme le dernier bandeau, sinon sur le premier contrôleur.
                    if let Some(c) = setup.fixtures.last().map(|f| f.controller).or(setup.controllers.first().map(|c| c.id)) {
                        f.controller = c;
                    }
                    if let Some(p) = at {
                        f.pos = [p[0], f.pos[1], p[2]];
                    }
                    setup.fixtures.push(f);
                    Some(format!("f{id}"))
                }
                "surface" => {
                    let (label, mut s) = Surface::presets(setup).into_iter().find(|(n, _)| *n == name)?;
                    if let Some(p) = at {
                        s.pos = [p[0], s.pos[1], p[2]];
                    }
                    s.id = setup.next_surface_id();
                    let n = setup.surfaces.iter().filter(|o| o.name.starts_with(label)).count() + 1;
                    s.name = format!("{label} {n}");
                    let id = s.id;
                    setup.surfaces.push(s);
                    Some(format!("s{id}"))
                }
                _ => None,
            }
        }
        RoomMsg::Dup { id } => {
            let (cat, n) = split(id)?;
            let shift = |p: [f32; 3]| [p[0] + 0.5, p[1], p[2]];
            match cat {
                'f' => {
                    let mut f = setup.fixtures.iter().find(|f| f.id == n)?.clone();
                    f.id = setup.next_fixture_id();
                    f.pos = shift(f.pos);
                    f.name = format!("{} (copie)", f.name);
                    f.set = 0;
                    let id = f.id;
                    setup.fixtures.push(f);
                    Some(format!("f{id}"))
                }
                's' => {
                    let mut s = setup.surfaces.iter().find(|s| s.id == n)?.clone();
                    s.id = setup.next_surface_id();
                    s.pos = shift(s.pos);
                    s.name = format!("{} (copie)", s.name);
                    s.set = 0;
                    let id = s.id;
                    setup.surfaces.push(s);
                    Some(format!("s{id}"))
                }
                // Les appareils DMX et les lasers ont une adresse : ils s'ajoutent sur l'ordinateur.
                _ => None,
            }
        }
        RoomMsg::Del { id } => {
            let (cat, n) = split(id)?;
            match cat {
                'f' => setup.fixtures.retain(|f| f.id != n),
                's' => setup.surfaces.retain(|s| s.id != n),
                'd' => setup.dmx.retain(|d| d.id != n),
                'l' => setup.lasers.retain(|l| l.id != n),
                _ => return None,
            }
            setup.prune_sets();
            None
        }
    }
}

fn split(id: &str) -> Option<(char, u32)> {
    let mut c = id.chars();
    let cat = c.next()?;
    Some((cat, c.as_str().parse().ok()?))
}

/// Position et orientation d'un objet, à modifier.
fn find<'a>(setup: &'a mut LedSetup, id: &str) -> Option<(&'a mut [f32; 3], &'a mut [f32; 3])> {
    let (cat, n) = split(id)?;
    match cat {
        'f' => setup.fixtures.iter_mut().find(|f| f.id == n).map(|f| (&mut f.pos, &mut f.rot)),
        's' => setup.surfaces.iter_mut().find(|s| s.id == n).map(|s| (&mut s.pos, &mut s.rot)),
        'd' => setup.dmx.iter_mut().find(|d| d.id == n).map(|d| (&mut d.pos, &mut d.rot)),
        'l' => setup.lasers.iter_mut().find(|l| l.id == n).map(|l| (&mut l.pos, &mut l.rot)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_decor_has_faces() {
        let setup = LedSetup::default();
        for (name, s) in Surface::presets(&setup) {
            let faces = s.faces(0.0);
            assert!(!faces.is_empty(), "{name} : aucune face");
            assert!(faces.iter().all(|f| f.quad.iter().flatten().all(|v| v.is_finite())), "{name} : point invalide");
            assert!(faces.len() < 20_000, "{name} : trop de faces ({})", faces.len());
        }
    }

    #[test]
    fn room_edits() {
        let mut setup = LedSetup::default();
        let before = setup.fixtures.len();
        let id = apply(&mut setup, &RoomMsg::Add { what: "fixture:Strip".into(), pos: Some([1.0, 0.0, 3.0]) }).unwrap();
        assert_eq!(setup.fixtures.len(), before + 1);
        let f = setup.fixtures.last().unwrap();
        assert_eq!((f.pos[0], f.pos[2]), (1.0, 3.0));
        apply(&mut setup, &RoomMsg::Place { id: id.clone(), pos: [2.0, 1.5, 4.0], yaw: Some(450.0) });
        let f = setup.fixtures.last().unwrap();
        assert_eq!(f.pos, [2.0, 1.5, 4.0]);
        assert_eq!(f.rot[0], 90.0);
        // Valeurs absurdes refusées.
        apply(&mut setup, &RoomMsg::Place { id: id.clone(), pos: [f32::NAN, 0.0, 0.0], yaw: None });
        assert_eq!(setup.fixtures.last().unwrap().pos, [2.0, 1.5, 4.0]);
        let copy = apply(&mut setup, &RoomMsg::Dup { id: id.clone() }).unwrap();
        assert_ne!(copy, id);
        apply(&mut setup, &RoomMsg::Del { id: copy });
        apply(&mut setup, &RoomMsg::Del { id });
        assert_eq!(setup.fixtures.len(), before);
        let s = apply(&mut setup, &RoomMsg::Add { what: "surface:Écran".into(), pos: None }).unwrap();
        assert!(s.starts_with('s'));
        let j: Value = serde_json::from_str(&room_json(&setup)).unwrap();
        assert_eq!(j["t"], "room");
        assert!(j["items"].as_array().unwrap().iter().any(|i| i["id"] == s.as_str()));
        let m: RoomMsg = serde_json::from_str(r#"{"op":"place","id":"f1","pos":[0,1,2],"yaw":30}"#).unwrap();
        assert!(matches!(m, RoomMsg::Place { yaw: Some(_), .. }));
    }
}
