//! Socle du monde vivant : `EntityStub` + cycle de vie LOD (spec
//! `2026-07-23-spec-implementation-serveur-monde-vivant.md` §1/§2, extrait RE §0/§4bis).
//!
//! Reproduit `gameEntityStubSystem` : chaque entité de foule/trafic/police/communauté existe
//! d'abord comme un STUB léger (position + orientation + archétype + état), promu en entité complète
//! (`Live`) près d'un joueur, démoté/expiré au loin. C'est le cœur de la fidélité solo (« ils
//! spawnent quand on ne les voit pas ») ET la borne de coût (seuls les `Live` sont tickés en détail).
//!
//! Découplage imposé par le RE (§4bis, `entity.state[0x64]==4 → disposal`) : la DÉCISION (passe
//! d'update qui transitionne les états) et l'ACTION (passe de disposal qui libère les `Expired`) sont
//! DEUX passes séparées. Le serveur ne détruit JAMAIS une entité dans la boucle de décision — comme
//! le moteur qui collecte d'abord les `state==4`, puis dispose dans une passe distincte. Ce module
//! est PUR (aucune I/O, aucun `.await`), à l'image de `npc.rs`/`population_director.rs` — l'appelant
//! (`server_loop`) possède le `StubPool` et applique les effets (placement, retrait du fil).
//!
//! Ce que ce module N'implémente PAS encore (incréments suivants du plan de code, spec §7) : le
//! PLACEMENT au spawn (les producteurs `CrowdProducer`/`TrafficProducer`/… qui décident OÙ et QUEL
//! archétype — spec §3), et le suivi de voies (`nav.rs`). Ici : uniquement le socle de cycle de vie.

use crate::npc::EntityBehavior;
use crate::transport::ClientId;

/// Ce qu'EST l'entité (rendu/simulation), distinct de QUI l'a produite (`ProducerId`). Le RE §0
/// distingue ces familles dans le stub system ; le client rend chacune différemment (piéton animé,
/// véhicule conduit, unité de police).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Pedestrian,
    Vehicle,
    CommunityMember,
    PoliceUnit,
}

/// QUI a créé le stub (spec §3 — chaque système du RE devient un producteur). Sert à router la passe
/// d'update vers les plafonds/règles propres au producteur (ex. `communityRunnersNumberLimit` vs
/// `trafficRunnersNumberLimit`, valeurs Phase 1 distinctes) et à savoir qui « re-produit » après une
/// vague de peur (`CommunitiesToRespawn`, spec §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProducerId {
    Crowd,
    Traffic,
    Community,
    Police,
}

/// Cycle de vie d'un stub (spec §1/§2). Le RE a montré un flag d'état par entité, valeur 4 = terminal
/// (`state[0x64]==4` → disposal) : on reproduit un enum EXPLICITE plutôt qu'un entier magique.
///
/// Le graphe autorisé (une transition non listée est un bug, cf. `update_lifecycle`) :
/// `Dormant → Materializing → Live → Expired → Disposing`. `Live` peut aussi rester `Live`
/// (dans/hors portée mais pas encore expiré). Aucun retour arrière : un `Expired` ne redevient
/// jamais `Live` (le RE dispose ; une entité re-nécessaire est un NOUVEAU stub promu). La démotion
/// `Live → Dormant` (garder le stub léger mais dé-matérialiser sans détruire) est un raffinement
/// futur — pour l'instant on expire puis on re-promeut, plus simple et fidèle au collect-then-dispose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Dormant,
    Materializing,
    Live,
    Expired,
    Disposing,
}

/// Constantes de portée/temps du cycle de vie, cueillies EN JEU le 2026-07-23 (TweakDB, cf.
/// `tools/reverse-engineering/tweakdb-valeurs.md`). Toutes les DISTANCES sont linéaires ici ; les
/// comparaisons internes se font au CARRÉ (le moteur compare distance² pour éviter un `sqrt` par
/// entité par tick — RE FINDINGS §3, « toutes au carré ») via `*_dist_sq()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldConfig {
    /// Rayon de PROMOTION (`Dormant → Materializing`). ⚠️ Le flat de rayon de spawn de la FOULE n'est
    /// pas dans les 81 clés cueillies Phase 1 (elles couvrent réactions + police) — cette valeur est
    /// un placeholder d'hystérésis (< `despawn_dist` pour ne pas osciller au bord) **à valider
    /// Phase 6** (mesure du spawn réel en solo). Ne pas la prendre pour une valeur RE confirmée.
    pub spawn_dist: f32,
    /// Rayon de DESPAWN (`despawningMaxDistance` = 45 m pour la foule liée aux réactions).
    pub despawn_dist: f32,
    /// Délai hors-portée avant expiration (`despawningMaxTime` = 10 s). Un stub `Live` dont TOUS les
    /// joueurs sont sortis du rayon de despawn n'expire qu'après ce délai (spec §2 : « distance ET
    /// temps ») — évite le clignotement d'une entité qu'un joueur frôle.
    pub despawn_time_ms: u64,
    /// Plancher du délai ci-dessus (`despawningMinTimeFactor` = 0.10 → 1 s mini pour 10 s). Ne mord
    /// que si `despawn_time_ms` est configuré très bas ; borne le délai effectif par le bas.
    pub despawn_min_time_ms: u64,
}

impl WorldConfig {
    /// Valeurs de la FOULE (réactions) telles que lues en jeu le 2026-07-23. `spawn_dist` reste un
    /// placeholder (voir champ). Les producteurs police/trafic ont leurs PROPRES distances
    /// (`vehicle_spawn_setup.*Despawn*` 90/150 m, `forcedDespawnDistance` 1000 m) — un `WorldConfig`
    /// distinct par producteur, pas une constante globale unique.
    pub fn crowd_defaults() -> Self {
        Self {
            spawn_dist: 40.0,           // placeholder < 45, à valider Phase 6
            despawn_dist: 45.0,         // despawningMaxDistance
            despawn_time_ms: 10_000,    // despawningMaxTime = 10 s
            despawn_min_time_ms: 1_000, // 10 s × 0.10 (despawningMinTimeFactor)
        }
    }

    fn spawn_dist_sq(&self) -> f32 {
        self.spawn_dist * self.spawn_dist
    }

    fn despawn_dist_sq(&self) -> f32 {
        self.despawn_dist * self.despawn_dist
    }

    /// Délai d'expiration effectif : le max du délai configuré et de son plancher (le plancher borne
    /// par le bas, il ne raccourcit jamais un délai déjà supérieur).
    fn effective_despawn_delay_ms(&self) -> u64 {
        self.despawn_time_ms.max(self.despawn_min_time_ms)
    }
}

/// État LÉGER d'une entité de monde vivant, persistant qu'elle soit matérialisée ou non (spec §1).
/// `transform` = position monde + `yaw` STOCKÉS : le RE montre que la promotion place l'entité au
/// transform mémorisé (pas une position recalculée). Le quaternion complet d'orientation est différé
/// (spec §6 : nécessaire aux véhicules, pas encore au fil) — `yaw` suffit pour un piéton.
#[derive(Debug, Clone)]
pub struct EntityStub {
    pub id: ClientId,
    pub kind: EntityKind,
    pub archetype: u32,
    pub position: (f32, f32, f32),
    pub yaw: f32,
    pub lifecycle: Lifecycle,
    pub producer: ProducerId,
    /// Comportement FSM (réutilise `EntityBehavior` de `npc.rs`) — pertinent seulement quand `Live`,
    /// mais porté par le stub pour survivre à une démotion future sans perdre l'état de peur/hostilité.
    pub behavior: EntityBehavior,
    /// Horodatage (ms) où ce stub `Live` est passé hors du rayon de despawn de TOUS les joueurs.
    /// `None` = au moins un joueur était à portée au dernier update (ou stub pas encore `Live`).
    /// Sert au prédicat « distance ET durée » : on n'expire qu'après `effective_despawn_delay_ms`
    /// hors-portée continue. Un joueur qui revient à portée le remet à `None` (le timer redémarre).
    out_of_range_since_ms: Option<u64>,
}

impl EntityStub {
    /// Crée un stub `Dormant` (connu mais pas matérialisé) — état de départ de tout stub produit.
    /// L'id DOIT être dans la plage réservée (`world::is_npc_id`) : garanti par le producteur, pas
    /// re-vérifié ici (ce module ne connaît pas la politique d'allocation d'ids).
    pub fn new_dormant(
        id: ClientId,
        kind: EntityKind,
        archetype: u32,
        position: (f32, f32, f32),
        yaw: f32,
        producer: ProducerId,
    ) -> Self {
        Self {
            id,
            kind,
            archetype,
            position,
            yaw,
            lifecycle: Lifecycle::Dormant,
            producer,
            behavior: EntityBehavior::default(),
            out_of_range_since_ms: None,
        }
    }

    /// Marque un stub `Materializing` comme placé → `Live`. Appelé par l'appelant APRÈS que l'entité
    /// complète a réellement été instanciée dans le monde (spec §2 : « `Materializing → Live` quand
    /// l'entité est placée »). Séparé de `update_lifecycle` car le placement est une ACTION externe
    /// (le module de décision ne place rien). No-op si le stub n'est pas `Materializing` (idempotent).
    pub fn mark_materialized(&mut self) {
        if self.lifecycle == Lifecycle::Materializing {
            self.lifecycle = Lifecycle::Live;
            self.out_of_range_since_ms = None;
        }
    }

    /// PASSE DE DÉCISION (par tick, par stub) : transitionne le cycle de vie selon la proximité du
    /// joueur le plus proche. `nearest_dist_sq` = distance² au joueur le plus proche du shard
    /// (`None` = aucun joueur — traité comme « hors de tout rayon »). `now_ms` : horloge de tick
    /// cohérente avec l'appelant. Retourne `true` si l'état a changé (pour le fil / métriques).
    ///
    /// Ne PLACE ni ne DÉTRUIT rien : `Dormant → Materializing` signale qu'il FAUT placer (l'appelant
    /// place puis appelle `mark_materialized`), `Live → Expired` signale qu'il FAUT disposer (la
    /// passe de disposal le fera). `Materializing` n'est jamais avancé ici (attend `mark_materialized`)
    /// ni annulé (l'annulation d'une matérialisation en cours, si le joueur repart, est un raffinement
    /// futur — documenté, pas un oubli). `Expired`/`Disposing` sont terminaux pour cette passe.
    pub fn update_lifecycle(
        &mut self,
        nearest_dist_sq: Option<f32>,
        now_ms: u64,
        cfg: &WorldConfig,
    ) -> bool {
        match self.lifecycle {
            Lifecycle::Dormant => {
                if within(nearest_dist_sq, cfg.spawn_dist_sq()) {
                    self.lifecycle = Lifecycle::Materializing;
                    true
                } else {
                    false
                }
            }
            Lifecycle::Live => {
                if within(nearest_dist_sq, cfg.despawn_dist_sq()) {
                    // Au moins un joueur est dans le rayon de despawn : le stub reste, timer annulé.
                    let changed = self.out_of_range_since_ms.is_some();
                    self.out_of_range_since_ms = None;
                    changed
                } else {
                    // Hors de portée de despawn de TOUS les joueurs : démarrer/consulter le timer.
                    match self.out_of_range_since_ms {
                        None => {
                            self.out_of_range_since_ms = Some(now_ms);
                            false // toujours Live, on vient juste d'armer le timer
                        }
                        Some(since) => {
                            if now_ms.saturating_sub(since) >= cfg.effective_despawn_delay_ms() {
                                self.lifecycle = Lifecycle::Expired;
                                true
                            } else {
                                false
                            }
                        }
                    }
                }
            }
            // Materializing : attend mark_materialized. Expired/Disposing : terminaux (la passe de
            // disposal s'en occupe). Aucune transition dans la passe de décision.
            Lifecycle::Materializing | Lifecycle::Expired | Lifecycle::Disposing => false,
        }
    }
}

/// `nearest_dist_sq <= threshold_sq` ? `None` (aucun joueur) => jamais à portée.
fn within(nearest_dist_sq: Option<f32>, threshold_sq: f32) -> bool {
    match nearest_dist_sq {
        Some(d2) => d2 <= threshold_sq,
        None => false,
    }
}

/// Le pool de stubs d'un shard : possède les stubs et expose les DEUX passes découplées (décision
/// puis disposal). L'appelant (`server_loop::tick`) le détient et applique les effets externes
/// (placer les `Materializing`, retirer du fil les `Disposing`). Volontairement une struct fine
/// au-dessus d'un `Vec` — la logique de fond vit dans `EntityStub`, testée unité par unité.
#[derive(Debug, Default)]
pub struct StubPool {
    pub stubs: Vec<EntityStub>,
}

impl StubPool {
    pub fn new() -> Self {
        Self { stubs: Vec::new() }
    }

    pub fn insert(&mut self, stub: EntityStub) {
        self.stubs.push(stub);
    }

    pub fn len(&self) -> usize {
        self.stubs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stubs.is_empty()
    }

    /// Nombre de stubs `Live` (les seuls tickés en détail / rendus). Sert au LOD et aux plafonds
    /// (`totalEntitiesLimit`, `*RunnersNumberLimit`).
    pub fn live_count(&self) -> usize {
        self.stubs
            .iter()
            .filter(|s| s.lifecycle == Lifecycle::Live)
            .count()
    }

    /// PASSE DE DÉCISION sur tout le pool. `player_positions` : positions monde des joueurs du shard.
    /// Pour chaque stub, calcule la distance² au joueur le plus proche et transitionne son cycle de
    /// vie. Retourne les ids des stubs devenus `Materializing` ce tick (l'appelant doit les PLACER
    /// puis appeler `mark_materialized`). Ne dispose de rien — c'est la passe séparée `disposal_pass`.
    pub fn update_pass(
        &mut self,
        player_positions: &[(f32, f32, f32)],
        now_ms: u64,
        cfg: &WorldConfig,
    ) -> Vec<ClientId> {
        let mut to_materialize = Vec::new();
        for stub in &mut self.stubs {
            let nearest_sq = nearest_dist_sq(stub.position, player_positions);
            let was_dormant = stub.lifecycle == Lifecycle::Dormant;
            stub.update_lifecycle(nearest_sq, now_ms, cfg);
            if was_dormant && stub.lifecycle == Lifecycle::Materializing {
                to_materialize.push(stub.id);
            }
        }
        to_materialize
    }

    /// PASSE DE DISPOSAL (séparée de la décision, RE §4bis collect-then-dispose). Collecte les stubs
    /// `Expired`, les fait passer à `Disposing`, RETIRE du pool, et retourne leurs ids pour que
    /// l'appelant les efface du fil/monde. Deux temps dans la même passe (marquer `Disposing` puis
    /// retirer) mais JAMAIS mêlé à la passe de décision — le serveur ne détruit rien pendant qu'il
    /// décide, exactement comme le moteur collecte les `state==4` avant de disposer.
    pub fn disposal_pass(&mut self) -> Vec<ClientId> {
        let mut disposed = Vec::new();
        for stub in &mut self.stubs {
            if stub.lifecycle == Lifecycle::Expired {
                stub.lifecycle = Lifecycle::Disposing;
                disposed.push(stub.id);
            }
        }
        // Retrait effectif APRÈS la collecte (jamais pendant qu'on itère pour décider).
        self.stubs.retain(|s| s.lifecycle != Lifecycle::Disposing);
        disposed
    }
}

/// Distance² au joueur le plus proche (plan XY — la verticalité de Night City n'entre pas dans le
/// rayon de streaming horizontal ; cohérent avec la grille XY de `world.rs`). `None` si aucun joueur.
fn nearest_dist_sq(pos: (f32, f32, f32), players: &[(f32, f32, f32)]) -> Option<f32> {
    players
        .iter()
        .map(|p| {
            let dx = pos.0 - p.0;
            let dy = pos.1 - p.1;
            dx * dx + dy * dy
        })
        .fold(None, |acc, d2| match acc {
            Some(m) if m <= d2 => Some(m),
            _ => Some(d2),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dormant_at(x: f32, y: f32) -> EntityStub {
        EntityStub::new_dormant(
            crate::world::NPC_ID_RANGE_START + 1,
            EntityKind::Pedestrian,
            216,
            (x, y, 0.0),
            0.0,
            ProducerId::Crowd,
        )
    }

    #[test]
    fn a_new_stub_starts_dormant() {
        assert_eq!(dormant_at(0.0, 0.0).lifecycle, Lifecycle::Dormant);
    }

    #[test]
    fn a_dormant_stub_promotes_when_a_player_enters_the_spawn_radius() {
        let cfg = WorldConfig::crowd_defaults();
        let mut s = dormant_at(0.0, 0.0);
        // Joueur à 30 m (< spawn_dist 40) -> distance² = 900.
        let changed = s.update_lifecycle(Some(30.0 * 30.0), 0, &cfg);
        assert!(changed);
        assert_eq!(s.lifecycle, Lifecycle::Materializing);
    }

    #[test]
    fn a_dormant_stub_stays_dormant_when_players_are_beyond_the_spawn_radius() {
        let cfg = WorldConfig::crowd_defaults();
        let mut s = dormant_at(0.0, 0.0);
        // Joueur à 50 m (> spawn_dist 40).
        let changed = s.update_lifecycle(Some(50.0 * 50.0), 0, &cfg);
        assert!(!changed);
        assert_eq!(s.lifecycle, Lifecycle::Dormant);
    }

    #[test]
    fn no_players_never_promotes() {
        let cfg = WorldConfig::crowd_defaults();
        let mut s = dormant_at(0.0, 0.0);
        assert!(!s.update_lifecycle(None, 0, &cfg));
        assert_eq!(s.lifecycle, Lifecycle::Dormant);
    }

    #[test]
    fn mark_materialized_promotes_materializing_to_live() {
        let cfg = WorldConfig::crowd_defaults();
        let mut s = dormant_at(0.0, 0.0);
        s.update_lifecycle(Some(100.0), 0, &cfg); // -> Materializing
        assert_eq!(s.lifecycle, Lifecycle::Materializing);
        s.mark_materialized();
        assert_eq!(s.lifecycle, Lifecycle::Live);
    }

    #[test]
    fn mark_materialized_is_a_noop_on_a_dormant_stub() {
        let mut s = dormant_at(0.0, 0.0);
        s.mark_materialized();
        assert_eq!(s.lifecycle, Lifecycle::Dormant);
    }

    /// Cœur du prédicat « distance ET durée » : un joueur qui sort du rayon de despawn n'expire pas
    /// l'entité tout de suite — il faut `despawn_time_ms` continu hors-portée.
    #[test]
    fn a_live_stub_expires_only_after_the_despawn_delay_out_of_range() {
        let cfg = WorldConfig::crowd_defaults(); // despawn 45 m, délai 10 s
        let mut s = dormant_at(0.0, 0.0);
        s.update_lifecycle(Some(100.0), 0, &cfg);
        s.mark_materialized(); // Live

        // t=0 : joueur à 100 m (> 45) -> arme le timer, reste Live.
        assert!(!s.update_lifecycle(Some(100.0 * 100.0), 0, &cfg));
        assert_eq!(s.lifecycle, Lifecycle::Live);

        // t=9 s : toujours hors-portée mais délai (10 s) pas atteint -> reste Live.
        assert!(!s.update_lifecycle(Some(100.0 * 100.0), 9_000, &cfg));
        assert_eq!(s.lifecycle, Lifecycle::Live);

        // t=10 s : délai atteint -> Expired.
        assert!(s.update_lifecycle(Some(100.0 * 100.0), 10_000, &cfg));
        assert_eq!(s.lifecycle, Lifecycle::Expired);
    }

    #[test]
    fn a_player_returning_within_range_resets_the_despawn_timer() {
        let cfg = WorldConfig::crowd_defaults();
        let mut s = dormant_at(0.0, 0.0);
        s.update_lifecycle(Some(100.0), 0, &cfg);
        s.mark_materialized(); // Live

        s.update_lifecycle(Some(100.0 * 100.0), 0, &cfg); // arme le timer à t=0
                                                          // Joueur revient à 10 m (< 45) à t=5 s -> timer annulé, reste Live.
        let changed = s.update_lifecycle(Some(10.0 * 10.0), 5_000, &cfg);
        assert!(
            changed,
            "l'annulation du timer est un changement d'état interne"
        );
        assert_eq!(s.lifecycle, Lifecycle::Live);

        // Repart hors-portée à t=6 s : le timer REDÉMARRE (pas de mémoire du premier). À t=15 s
        // (9 s après le redémarrage, < 10 s) il ne doit PAS encore expirer.
        s.update_lifecycle(Some(100.0 * 100.0), 6_000, &cfg);
        assert!(!s.update_lifecycle(Some(100.0 * 100.0), 15_000, &cfg));
        assert_eq!(s.lifecycle, Lifecycle::Live);
        // À t=16 s (10 s après le redémarrage) -> Expired.
        assert!(s.update_lifecycle(Some(100.0 * 100.0), 16_000, &cfg));
        assert_eq!(s.lifecycle, Lifecycle::Expired);
    }

    #[test]
    fn a_live_stub_with_a_nearby_player_never_expires() {
        let cfg = WorldConfig::crowd_defaults();
        let mut s = dormant_at(0.0, 0.0);
        s.update_lifecycle(Some(100.0), 0, &cfg);
        s.mark_materialized();
        for t in 0..100 {
            s.update_lifecycle(Some(10.0 * 10.0), t * 1_000, &cfg); // toujours à 10 m
        }
        assert_eq!(s.lifecycle, Lifecycle::Live);
    }

    #[test]
    fn effective_delay_is_floored_by_the_min_time_factor() {
        // despawn_time_ms très bas (0) mais plancher 1 s : l'expiration ne doit pas être instantanée.
        let cfg = WorldConfig {
            spawn_dist: 40.0,
            despawn_dist: 45.0,
            despawn_time_ms: 0,
            despawn_min_time_ms: 1_000,
        };
        let mut s = dormant_at(0.0, 0.0);
        s.update_lifecycle(Some(100.0), 0, &cfg);
        s.mark_materialized();
        s.update_lifecycle(Some(100.0 * 100.0), 0, &cfg); // arme
        assert!(!s.update_lifecycle(Some(100.0 * 100.0), 500, &cfg)); // 0.5 s < 1 s plancher
        assert_eq!(s.lifecycle, Lifecycle::Live);
        assert!(s.update_lifecycle(Some(100.0 * 100.0), 1_000, &cfg)); // 1 s = plancher
        assert_eq!(s.lifecycle, Lifecycle::Expired);
    }

    // ---- StubPool : les deux passes découplées ----

    #[test]
    fn update_pass_reports_stubs_to_materialize_and_disposal_pass_removes_expired() {
        let cfg = WorldConfig::crowd_defaults();
        let mut pool = StubPool::new();
        // Deux stubs : un proche (0,0) qui va se matérialiser, un loin (1000,1000) qui reste dormant.
        pool.insert(EntityStub::new_dormant(
            crate::world::NPC_ID_RANGE_START + 1,
            EntityKind::Pedestrian,
            216,
            (0.0, 0.0, 0.0),
            0.0,
            ProducerId::Crowd,
        ));
        pool.insert(EntityStub::new_dormant(
            crate::world::NPC_ID_RANGE_START + 2,
            EntityKind::Pedestrian,
            216,
            (1000.0, 1000.0, 0.0),
            0.0,
            ProducerId::Crowd,
        ));

        // Joueur à l'origine.
        let players = [(0.0, 0.0, 0.0)];
        let to_mat = pool.update_pass(&players, 0, &cfg);
        assert_eq!(to_mat, vec![crate::world::NPC_ID_RANGE_START + 1]);
        assert_eq!(pool.len(), 2, "aucune disposal dans la passe de décision");

        // L'appelant place et matérialise le proche.
        for s in &mut pool.stubs {
            if to_mat.contains(&s.id) {
                s.mark_materialized();
            }
        }
        assert_eq!(pool.live_count(), 1);

        // Le joueur part très loin : après le délai, le Live expire.
        let far = [(100_000.0, 100_000.0, 0.0)];
        pool.update_pass(&far, 0, &cfg); // arme le timer
        pool.update_pass(&far, 10_000, &cfg); // expire

        // Passe de disposal : retire l'Expired, retourne son id.
        let disposed = pool.disposal_pass();
        assert_eq!(disposed, vec![crate::world::NPC_ID_RANGE_START + 1]);
        assert_eq!(pool.len(), 1, "seul le dormant lointain subsiste");
    }

    #[test]
    fn disposal_pass_is_empty_when_nothing_expired() {
        let mut pool = StubPool::new();
        pool.insert(dormant_at(0.0, 0.0));
        assert!(pool.disposal_pass().is_empty());
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn nearest_dist_sq_picks_the_closest_player() {
        let d = nearest_dist_sq((0.0, 0.0, 0.0), &[(100.0, 0.0, 0.0), (3.0, 4.0, 0.0)]);
        assert_eq!(d, Some(25.0)); // le second joueur, à 5 m -> 25
    }

    #[test]
    fn nearest_dist_sq_is_none_without_players() {
        assert_eq!(nearest_dist_sq((0.0, 0.0, 0.0), &[]), None);
    }
}
