//! Fusion de snapshots (M4) : un joueur en zone tampon reçoit un snapshot de chaque shard chargé.
//! Le Gateway les fusionne en un seul (union des joueurs par id) avant de l'envoyer au client —
//! le format reste l'`ServerEnvelope/Snapshot` que le client comprend déjà.

use flatbuffers::FlatBufferBuilder;
use protocol::*;
use std::collections::BTreeMap;

/// Champs d'un `VehicleState` (protocol.fbs) hors id, agrégés par id durant la fusion — miroir du
/// tuple position+yaw utilisé pour les joueurs, mais avec les champs propres au véhicule.
type VehicleAgg = (u32, f32, f32, f32, f32, u16, u64);

/// Unionne plusieurs snapshots serveur en un seul (dédup par id de joueur, `tick` = max).
/// `None` si aucun snapshot valide n'a pu être décodé.
pub fn merge_snapshots(snapshots: &[Vec<u8>]) -> Option<Vec<u8>> {
    let mut by_id: BTreeMap<u64, (f32, f32, f32, f32)> = BTreeMap::new();
    let mut vehicles_by_id: BTreeMap<u64, VehicleAgg> = BTreeMap::new();
    let mut tick: u64 = 0;
    let mut any = false;

    for bytes in snapshots {
        let Ok(env) = flatbuffers::root::<ServerEnvelope>(bytes) else {
            continue;
        };
        let Some(snap) = env.msg_as_snapshot() else {
            continue;
        };
        any = true;
        tick = tick.max(snap.tick());
        if let Some(players) = snap.players() {
            for p in players.iter() {
                if let Some(pos) = p.position() {
                    by_id
                        .entry(p.id())
                        .or_insert((pos.x(), pos.y(), pos.z(), p.yaw()));
                }
            }
        }
        if let Some(vehicles) = snap.vehicles() {
            for v in vehicles.iter() {
                if let Some(pos) = v.position() {
                    vehicles_by_id.entry(v.id()).or_insert((
                        v.archetype(),
                        pos.x(),
                        pos.y(),
                        pos.z(),
                        v.yaw(),
                        v.speed(),
                        v.passenger(),
                    ));
                }
            }
        }
    }

    if !any {
        return None;
    }

    let mut b = FlatBufferBuilder::new();
    let states: Vec<_> = by_id
        .iter()
        .map(|(id, (x, y, z, yaw))| {
            let pos = Vec3::new(*x, *y, *z);
            PlayerState::create(
                &mut b,
                &PlayerStateArgs {
                    id: *id,
                    position: Some(&pos),
                    yaw: *yaw,
                    locomotion: 0,
                    move_dir: 0,
                    flags: 0,
                    sustained: 0,
                },
            )
        })
        .collect();
    let players = b.create_vector(&states);

    let vehicle_states: Vec<_> = vehicles_by_id
        .iter()
        .map(|(id, (archetype, x, y, z, yaw, speed, passenger))| {
            let pos = Vec3::new(*x, *y, *z);
            VehicleState::create(
                &mut b,
                &VehicleStateArgs {
                    id: *id,
                    archetype: *archetype,
                    position: Some(&pos),
                    yaw: *yaw,
                    speed: *speed,
                    passenger: *passenger,
                },
            )
        })
        .collect();
    let vehicles_vec = if vehicle_states.is_empty() {
        None
    } else {
        Some(b.create_vector(&vehicle_states))
    };

    let snap = Snapshot::create(
        &mut b,
        &SnapshotArgs {
            tick,
            players: Some(players),
            // Les PNJ n'ont toujours pas de présence en zone tampon multi-shard (hors périmètre de
            // cette tâche — cf. commentaire d'origine, un futur chantier symétrique à celui-ci
            // pourrait un jour faire pour `npcs` ce que cette tâche fait pour `vehicles`).
            npcs: None,
            vehicles: vehicles_vec,
        },
    );
    let env = ServerEnvelope::create(
        &mut b,
        &ServerEnvelopeArgs {
            msg_type: ServerMsg::Snapshot,
            msg: Some(snap.as_union_value()),
        },
    );
    b.finish(env, None);
    Some(b.finished_data().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatbuffers::FlatBufferBuilder;

    /// Encode un Snapshot serveur (comme un shard l'enverrait) avec les joueurs donnés.
    fn snapshot(tick: u64, players: &[(u64, f32)]) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let states: Vec<_> = players
            .iter()
            .map(|(id, x)| {
                let pos = Vec3::new(*x, 0.0, 0.0);
                PlayerState::create(
                    &mut b,
                    &PlayerStateArgs {
                        id: *id,
                        position: Some(&pos),
                        yaw: 0.0,
                        locomotion: 0,
                        move_dir: 0,
                        flags: 0,
                        sustained: 0,
                    },
                )
            })
            .collect();
        let pv = b.create_vector(&states);
        let snap = Snapshot::create(
            &mut b,
            &SnapshotArgs {
                tick,
                players: Some(pv),
                npcs: None,
                vehicles: None,
            },
        );
        let env = ServerEnvelope::create(
            &mut b,
            &ServerEnvelopeArgs {
                msg_type: ServerMsg::Snapshot,
                msg: Some(snap.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    fn players_of(bytes: &[u8]) -> Vec<(u64, f32)> {
        let env = flatbuffers::root::<ServerEnvelope>(bytes).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let mut v: Vec<(u64, f32)> = snap
            .players()
            .unwrap()
            .iter()
            .map(|p| (p.id(), p.position().unwrap().x()))
            .collect();
        v.sort_by_key(|(id, _)| *id);
        v
    }

    #[test]
    fn merges_disjoint_players_and_takes_max_tick() {
        let a = snapshot(10, &[(1, 5.0)]);
        let b = snapshot(12, &[(2, 9.0)]);
        let merged = merge_snapshots(&[a, b]).unwrap();
        assert_eq!(players_of(&merged), vec![(1, 5.0), (2, 9.0)]);
        let env = flatbuffers::root::<ServerEnvelope>(&merged).unwrap();
        assert_eq!(env.msg_as_snapshot().unwrap().tick(), 12);
    }

    #[test]
    fn deduplicates_player_present_on_both_shards() {
        let a = snapshot(1, &[(1, 5.0), (3, 7.0)]);
        let b = snapshot(1, &[(1, 5.0)]); // joueur 1 sur les deux
        let merged = merge_snapshots(&[a, b]).unwrap();
        assert_eq!(players_of(&merged), vec![(1, 5.0), (3, 7.0)]);
    }

    #[test]
    fn returns_none_when_no_valid_snapshot() {
        assert!(merge_snapshots(&[]).is_none());
        assert!(merge_snapshots(&[vec![0, 1, 2]]).is_none());
    }

    /// Encode un Snapshot serveur avec, en plus des joueurs, un unique véhicule optionnel
    /// (id, archetype, x, y, z, speed, passenger) — suffisant pour ces tests, pas besoin de vecteur.
    fn snapshot_with_vehicle(
        tick: u64,
        players: &[(u64, f32)],
        vehicle: Option<(u64, u32, f32, f32, f32, u16, u64)>,
    ) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let states: Vec<_> = players
            .iter()
            .map(|(id, x)| {
                let pos = Vec3::new(*x, 0.0, 0.0);
                PlayerState::create(
                    &mut b,
                    &PlayerStateArgs {
                        id: *id,
                        position: Some(&pos),
                        yaw: 0.0,
                        locomotion: 0,
                        move_dir: 0,
                        flags: 0,
                        sustained: 0,
                    },
                )
            })
            .collect();
        let pv = b.create_vector(&states);
        let vehicles_vec = vehicle.map(|(id, archetype, x, y, z, speed, passenger)| {
            let pos = Vec3::new(x, y, z);
            let v = VehicleState::create(
                &mut b,
                &VehicleStateArgs {
                    id,
                    archetype,
                    position: Some(&pos),
                    yaw: 0.0,
                    speed,
                    passenger,
                },
            );
            b.create_vector(&[v])
        });
        let snap = Snapshot::create(
            &mut b,
            &SnapshotArgs {
                tick,
                players: Some(pv),
                npcs: None,
                vehicles: vehicles_vec,
            },
        );
        let env = ServerEnvelope::create(
            &mut b,
            &ServerEnvelopeArgs {
                msg_type: ServerMsg::Snapshot,
                msg: Some(snap.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    /// Lit les véhicules d'un Snapshot fusionné, triés par id (comme `players_of`).
    fn vehicles_of(bytes: &[u8]) -> Vec<(u64, u32, f32, f32, f32, u16, u64)> {
        let env = flatbuffers::root::<ServerEnvelope>(bytes).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let mut v: Vec<(u64, u32, f32, f32, f32, u16, u64)> = snap
            .vehicles()
            .map(|vs| {
                vs.iter()
                    .map(|veh| {
                        let pos = veh.position().unwrap();
                        (
                            veh.id(),
                            veh.archetype(),
                            pos.x(),
                            pos.y(),
                            pos.z(),
                            veh.speed(),
                            veh.passenger(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by_key(|(id, ..)| *id);
        v
    }

    #[test]
    fn a_vehicle_present_on_one_shard_survives_the_merge() {
        let a = snapshot_with_vehicle(10, &[(1, 5.0)], Some((100, 3, 12.0, 0.0, 0.0, 42, 0)));
        let b = snapshot(12, &[(2, 9.0)]); // pas de véhicule sur ce snapshot
        let merged = merge_snapshots(&[a, b]).unwrap();
        let vehicles = vehicles_of(&merged);
        assert_eq!(vehicles, vec![(100, 3, 12.0, 0.0, 0.0, 42, 0)]);
    }

    #[test]
    fn deduplicates_vehicle_present_on_both_shards_keeping_the_first_seen() {
        // Valeurs délibérément DIFFÉRENTES entre les deux sources (id identique) pour distinguer
        // "premier snapshot vu gagne" (comme les joueurs, `.or_insert`) de "dernier gagne" — un
        // test avec deux tuples identiques ne prouverait que le dédoublonnage, pas la règle de
        // priorité. En pratique un véhicule n'est simulé que par un seul shard autoritaire à la
        // fois, donc ce désaccord ne devrait jamais survenir — ce test fige quand même le
        // comportement déterministe attendu si ça arrivait.
        let a = snapshot_with_vehicle(1, &[], Some((100, 3, 12.0, 0.0, 0.0, 42, 0)));
        let b = snapshot_with_vehicle(1, &[], Some((100, 3, 99.0, 0.0, 0.0, 42, 0)));
        let merged = merge_snapshots(&[a, b]).unwrap();
        let vehicles = vehicles_of(&merged);
        assert_eq!(vehicles.len(), 1, "un seul véhicule après dédoublonnage");
        assert_eq!(
            vehicles[0],
            (100, 3, 12.0, 0.0, 0.0, 42, 0),
            "le premier snapshot source vu doit gagner, pas le dernier"
        );
    }

    #[test]
    fn merge_with_no_vehicles_anywhere_produces_no_vehicles_vector_not_an_empty_one() {
        let a = snapshot(1, &[(1, 5.0)]); // pas de véhicule
        let merged = merge_snapshots(&[a]).unwrap();
        let env = flatbuffers::root::<ServerEnvelope>(&merged).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert!(
            snap.vehicles().is_none(),
            "l'absence de véhicule doit produire vehicles=None, pas Some(vec![])"
        );
    }
}
