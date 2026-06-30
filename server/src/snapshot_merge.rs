//! Fusion de snapshots (M4) : un joueur en zone tampon reçoit un snapshot de chaque shard chargé.
//! Le Gateway les fusionne en un seul (union des joueurs par id) avant de l'envoyer au client —
//! le format reste l'`ServerEnvelope/Snapshot` que le client comprend déjà.

use flatbuffers::FlatBufferBuilder;
use protocol::*;
use std::collections::BTreeMap;

/// Unionne plusieurs snapshots serveur en un seul (dédup par id de joueur, `tick` = max).
/// `None` si aucun snapshot valide n'a pu être décodé.
pub fn merge_snapshots(snapshots: &[Vec<u8>]) -> Option<Vec<u8>> {
    let mut by_id: BTreeMap<u64, (f32, f32, f32, f32)> = BTreeMap::new();
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
                },
            )
        })
        .collect();
    let players = b.create_vector(&states);
    let snap = Snapshot::create(
        &mut b,
        &SnapshotArgs {
            tick,
            players: Some(players),
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
}
