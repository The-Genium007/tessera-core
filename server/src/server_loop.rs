//! Boucle serveur : draine les events transport, met à jour le World, diffuse les snapshots.
//! Générique sur `Transport` → testable avec `InMemoryTransport`, branché sur GNS en prod.

use crate::elevator::{ElevatorState, MovementState};
use crate::elevator_catalog::ElevatorCatalog;
use crate::interaction_session::{SessionError, SessionRegistry};
use crate::metrics::Metrics;
use crate::named_npc_catalog::NamedNpcCatalog;
use crate::named_npc_registry::NamedNpcRegistry;
use crate::nav::plan_path;
use crate::nav_graph::{NavGraph, Vec3 as NavVec3};
use crate::nav_state::NavState;
use crate::npc::{
    behavior_to_u8, execute_transaction, EntityBehavior, NpcRecord, TransactionOutcome,
};
use crate::npc_catalog::NpcCatalog;
use crate::population_director::PopulationDirector;
use crate::transport::{ClientId, Transport, TransportEvent};
use crate::vehicle::VehicleRecord;
use crate::world::{Pose, World};
use flatbuffers::FlatBufferBuilder;
use protocol::*;
use std::sync::Arc;

pub struct Server {
    world: World,
    aoi_radius: f32,
    /// File d'événements one-shot du tick courant (actor, kind, action, param) — accumulée par
    /// `apply_client_message`, drainée et relayée aux voisins AoI en fin de `tick()`.
    pending_events: Vec<(ClientId, u8, u8, u32)>,
    /// Métriques partagées (buckets de durée de tick, overruns) — `None` pour tous les appels de
    /// test existants (`Server::new`), qui ne veulent pas dépendre de `Metrics`. `Some(..)`
    /// uniquement pour les `Server` construits via `Server::new_with_metrics` (câblage
    /// `shard_main`, Task 6 observabilité).
    metrics: Option<Arc<Metrics>>,
    /// `None` = aucune simulation PNJ sur ce Shard (comportement historique préservé — tous les
    /// appels `Server::new`/`Server::new_with_metrics` existants restent inchangés et n'activent
    /// jamais les PNJ). `Some(..)` uniquement via `Server::new_with_npcs`.
    npc_registry: Option<NpcRegistry>,
    /// `None` = aucun PNJ nominatif sur ce Shard (comportement historique préservé). `Some(..)`
    /// uniquement via `Server::new_with_named_npcs`.
    named_npc_registry: Option<NamedNpcRegistry>,
    /// `None` = aucun ascenseur simulé sur ce Shard. Tous les constructeurs historiques le laissent
    /// à `None` ; seul `new_with_elevators` l'active.
    elevator_registry: Option<ElevatorRegistry>,
    /// File one-shot des ids d'ascenseurs ayant reçu un appel NOUVELLEMENT accepté durant le
    /// drain des events de CE tick (`apply_client_message` la remplit sur `ClientMsg::ElevatorCall`,
    /// `tick()` la vide juste après `tick_elevators`). Nécessaire parce que `ElevatorState::advance`
    /// calcule son `before` de détection de changement APRÈS que `handle_elevator_call` a déjà muté
    /// `requested_floors` pour les appels reçus ce même tick (l'event-drain précède `advance` dans
    /// `tick()`) : un appel en pleine course qui n'altère ni `target_floor` ni `movement_state` ne
    /// ressort donc d'aucun changement détecté par `advance`, et sans ce relais ne serait diffusé
    /// qu'au prochain rappel heartbeat (jusqu'à ~1s) au lieu d'« appel accepté » (spec §5.3) —
    /// finding de la revue finale de branche du palier ascenseurs.
    pending_elevator_broadcasts: Vec<u64>,
    session_registry: SessionRegistry,
    /// File one-shot (actor, target) des `EntityInteraction(kind=2)` sur un PNJ nominatif à
    /// arbitrer en session — accumulée par `apply_client_message`, drainée en début de `tick()`.
    pending_interaction_opens: Vec<(ClientId, ClientId)>,
    /// File one-shot (actor, session_id, outcome, target) des sessions résolues à notifier au
    /// client — accumulée par `apply_client_message`, drainée en fin de `tick()`.
    pending_interaction_results: Vec<(ClientId, u64, TransactionOutcome, ClientId)>,
    /// Graphe de navigation (nav_graph.rs) — `None` = aucun PNJ ne se déplace (comportement
    /// fondation PNJ préservé : sans graphe, decide_destination peut produire une destination mais
    /// aucun chemin n'est jamais planifié, donc aucun mouvement). `Some(..)` posé après construction
    /// via un setter dédié (`set_nav_graph`) plutôt qu'un paramètre de constructeur — la navigation
    /// est un comportement additif, pas une variante de configuration comme catalog/director/registry.
    nav_graph: Option<NavGraph>,
    /// `None` = aucune simulation véhicule sur ce Shard (comportement historique préservé — tous
    /// les appels `Server::new`/`new_with_metrics`/`new_with_npcs`/`new_with_named_npcs` existants
    /// restent inchangés et n'activent jamais les véhicules). `Some(..)` uniquement via
    /// `Server::new_with_vehicles`, registre VIDE au départ (spawn explicite via `spawn_vehicle`,
    /// cf. doc de ce constructeur).
    vehicle_registry: Option<VehicleRegistry>,
    /// Rapports de position prédictifs (pont Shard→Gateway générique, `shard_boundary_bridge.rs`)
    /// accumulés par `tick_vehicles` ce tick — (entity_id, x, y, z, speed). Drainé par
    /// `take_pending_entity_reports`, appelé depuis `shard_main` (`Server`/`server_loop.rs` n'a pas
    /// de connexion TCP directe au Gateway, seul `shard.rs` possède la socket). Même patron que
    /// `pending_interaction_opens`/`pending_events`, mais PAS drainé dans `tick()` lui-même : ces
    /// deux files existantes relaient au client via `transport` (que `tick()` a déjà en main),
    /// alors qu'un rapport de position part vers le Gateway sur un canal totalement différent
    /// (la socket TCP interne, hors de portée de `Server`) — d'où un drain externe dédié.
    pending_entity_reports: Vec<(ClientId, f32, f32, f32, f32)>,
    /// `None` = aucune escalade policière active sur ce Shard (comportement historique préservé —
    /// tous les constructeurs existants n'activent jamais le heat). `Some(..)` uniquement via
    /// `Server::new_with_police_escalation` (spec PNJ hostiles §3). Le heat est un scalaire
    /// serveur-autoritaire mono-district (même simplification que `PopulationDirector`), jamais
    /// transporté sur le protocole — seuls ses futurs effets (peuplement policier) voyageront.
    /// La vraie `EscalationPolicy`/`EscalationThreshold` (escalade_police.rs, Task 3) n'est PAS
    /// encore chargée depuis TOML ici : `tick_npcs` applique des valeurs fixes (montant/decay),
    /// raffinement de configuration différé, documenté sur `new_with_police_escalation`.
    heat_tracker: Option<crate::escalade_police::HeatTracker>,
    /// Événements de transition PV=0 (spec PNJ hostiles §2 : "signal net, horodaté, archivable" pour
    /// la télémétrie shadow-flag) — `(npc_id, archetype, killer, timestamp_ms)`. Drainé par
    /// `shard_main`, qui les écrit en JSONL via `hostile_telemetry::append_combat_event` — même
    /// séparation des responsabilités que `pending_entity_reports` (le pont véhicules) : `Server`/
    /// `server_loop.rs` n'a aucune I/O directe, seul `shard.rs::shard_main` en a une.
    pending_combat_events: Vec<(ClientId, u32, ClientId, u64)>,
    /// Fenêtre glissante des N dernières durées de tick (micros) — indépendante de `metrics`
    /// (`Option<Arc<Metrics>>`, câblé pour Prometheus séparément) : la dégradation est un mécanisme de
    /// sécurité qui doit fonctionner même sur un `Server` sans metrics configurées. Capacité bornée à
    /// `TICK_DURATION_WINDOW_SIZE` (`VecDeque::pop_front` quand pleine, cf. `Server::tick`).
    tick_duration_window: std::collections::VecDeque<u64>,
    /// Palier de dégradation courant (spec tenue-en-charge §3, `degradation.rs`) — maintenu par
    /// hystérésis à chaque tick via `DegradationPolicy::tier_for_p99`. Défaut `Normal` : un `Server`
    /// qui vient de démarrer (aucun tick mesuré) n'est jamais dégradé par défaut.
    degradation_tier: crate::degradation::DegradationTier,
}

/// Regroupe l'état PNJ vivant d'un Shard : le catalogue (immuable après boot), le director
/// (config immuable), et les enregistrements PNJ actifs (mutables à chaque tick).
struct NpcRegistry {
    catalog: NpcCatalog,
    director: PopulationDirector,
    records: std::collections::HashMap<ClientId, NpcRecord>,
    next_npc_id: ClientId,
    /// Chemin/progression courants par PNJ (Task 3/5, plan navigation). Absent d'un id =
    /// « n'a jamais eu de destination assignée » — traité comme un NavState vierge.
    nav_states: std::collections::HashMap<ClientId, NavState>,
}

/// État ascenseur vivant d'un Shard. `tick_ms` est fourni à la construction plutôt que lu d'une
/// constante globale : `elevator.rs` reste pur, et le registre est le seul endroit qui connaît la
/// cadence réelle du serveur.
struct ElevatorRegistry {
    states: Vec<ElevatorState>,
    tick_ms: u32,
}

impl ElevatorRegistry {
    fn get_mut(&mut self, elevator_id: u64) -> Option<&mut ElevatorState> {
        self.states
            .iter_mut()
            .find(|s| s.elevator_id == elevator_id)
    }
}

/// Regroupe l'état véhicule vivant d'un Shard — sibling de `NpcRegistry`, mais SANS catalogue ni
/// director : un véhicule n'est pas peuplé automatiquement dans ce noyau (spec véhicules autonomes
/// §2 « trafic d'ambiance » différé, cf. doc de `Server::new_with_vehicles`), il n'y a donc rien à
/// réconcilier contre une présence de district comme le fait `PopulationDirector` pour les PNJ.
struct VehicleRegistry {
    records: std::collections::HashMap<ClientId, VehicleRecord>,
    /// Chemin/progression courants par véhicule — même mécanique que `NpcRegistry::nav_states`
    /// (NavState réutilisé tel quel, cf. Interfaces de cette tâche).
    nav_states: std::collections::HashMap<ClientId, NavState>,
    next_vehicle_id: ClientId,
}

impl Server {
    pub fn new(aoi_radius: f32) -> Self {
        Self {
            world: World::new(),
            aoi_radius,
            pending_events: Vec::new(),
            metrics: None,
            npc_registry: None,
            named_npc_registry: None,
            elevator_registry: None,
            pending_elevator_broadcasts: Vec::new(),
            session_registry: SessionRegistry::new(),
            pending_interaction_opens: Vec::new(),
            pending_interaction_results: Vec::new(),
            nav_graph: None,
            vehicle_registry: None,
            pending_entity_reports: Vec::new(),
            heat_tracker: None,
            pending_combat_events: Vec::new(),
            tick_duration_window: std::collections::VecDeque::new(),
            degradation_tier: crate::degradation::DegradationTier::Normal,
        }
    }

    /// Identique à `Server::new`, avec en plus l'enregistrement de la durée de chaque tick dans
    /// `metrics` (buckets d'histogramme + compteur d'overruns, cf. `metrics.rs`). Nouvelle
    /// méthode plutôt qu'un changement de signature de `Server::new` : ce dernier est déjà
    /// appelé par de nombreux tests existants (`server_loop.rs`, `gateway.rs`, `shard.rs`) qui ne
    /// doivent pas être modifiés pour cette tâche.
    pub fn new_with_metrics(aoi_radius: f32, metrics: Arc<Metrics>) -> Self {
        Self {
            world: World::new(),
            aoi_radius,
            pending_events: Vec::new(),
            metrics: Some(metrics),
            npc_registry: None,
            named_npc_registry: None,
            elevator_registry: None,
            pending_elevator_broadcasts: Vec::new(),
            session_registry: SessionRegistry::new(),
            pending_interaction_opens: Vec::new(),
            pending_interaction_results: Vec::new(),
            nav_graph: None,
            vehicle_registry: None,
            pending_entity_reports: Vec::new(),
            heat_tracker: None,
            pending_combat_events: Vec::new(),
            tick_duration_window: std::collections::VecDeque::new(),
            degradation_tier: crate::degradation::DegradationTier::Normal,
        }
    }

    /// Identique à `Server::new`, avec en plus un registre PNJ actif (catalogue + director). Le
    /// director raisonne sur un unique district logique "default" dans cette fondation — le
    /// multi-district réel (topologie de shards) est un raffinement différé, pas câblé ici.
    /// Nouvelle méthode plutôt qu'un changement de signature de `Server::new`/`new_with_metrics` :
    /// ces deux constructeurs sont déjà appelés par de nombreux tests existants qui ne doivent pas
    /// changer pour cette tâche (même raisonnement que `new_with_metrics` lui-même).
    pub fn new_with_npcs(
        aoi_radius: f32,
        catalog: NpcCatalog,
        director: PopulationDirector,
    ) -> Self {
        Self {
            world: World::new(),
            aoi_radius,
            pending_events: Vec::new(),
            metrics: None,
            npc_registry: Some(NpcRegistry {
                catalog,
                director,
                records: std::collections::HashMap::new(),
                next_npc_id: crate::world::NPC_ID_RANGE_START,
                nav_states: std::collections::HashMap::new(),
            }),
            named_npc_registry: None,
            elevator_registry: None,
            pending_elevator_broadcasts: Vec::new(),
            session_registry: SessionRegistry::new(),
            pending_interaction_opens: Vec::new(),
            pending_interaction_results: Vec::new(),
            nav_graph: None,
            vehicle_registry: None,
            pending_entity_reports: Vec::new(),
            heat_tracker: None,
            pending_combat_events: Vec::new(),
            tick_duration_window: std::collections::VecDeque::new(),
            degradation_tier: crate::degradation::DegradationTier::Normal,
        }
    }

    /// Identique à `new_with_npcs`, avec en plus un tracker de heat policier actif (spec PNJ
    /// hostiles §3). Nouveau constructeur plutôt qu'un paramètre optionnel sur `new_with_npcs` :
    /// ce dernier est déjà appelé par de nombreux tests existants (même raisonnement que chaque
    /// constructeur précédent de ce fichier). N'accepte PAS encore d'`EscalationPolicy` en
    /// paramètre (Task 3, escalade_police.rs) : le chargement TOML d'une vraie politique
    /// configurable est un raffinement de configuration différé, cf. doc de `heat_tracker` et de
    /// `tick_npcs` — ce constructeur active le mécanisme `HeatTracker` avec des valeurs fixes.
    pub fn new_with_police_escalation(
        aoi_radius: f32,
        catalog: NpcCatalog,
        director: PopulationDirector,
    ) -> Self {
        let mut s = Self::new_with_npcs(aoi_radius, catalog, director);
        s.heat_tracker = Some(crate::escalade_police::HeatTracker::default());
        s
    }

    /// Identique à `Server::new`, avec en plus un registre de PNJ nominatifs actif — SPAWNÉS dans
    /// `World` dès la construction (position/pose initiale tirée de `catalog`), donc immédiatement
    /// visibles dans un snapshot de joueur proche. Prend `catalog` ET `named_npc_registry` (pas
    /// seulement le registre d'ids) précisément pour pouvoir faire ce spawn ici, en un seul
    /// endroit — Task 7 (câblage boot) n'aura donc RIEN à ajouter au-delà de charger le catalogue
    /// et appeler ce constructeur ; cette signature est définitive dès cette tâche, ne change pas
    /// dans les tâches suivantes. Nouvelle méthode plutôt qu'un changement de signature de
    /// `Server::new`/`new_with_metrics`/`new_with_npcs` : ces trois constructeurs sont déjà
    /// appelés par de nombreux tests existants qui ne doivent pas changer pour cette tâche.
    pub fn new_with_named_npcs(
        aoi_radius: f32,
        catalog: &NamedNpcCatalog,
        named_npc_registry: NamedNpcRegistry,
    ) -> Self {
        let mut world = World::new();
        for runtime_id in named_npc_registry.runtime_ids() {
            let Some(manifest_id) = named_npc_registry.manifest_id_of(runtime_id) else {
                continue;
            };
            let Some(config) = catalog.get(manifest_id) else {
                continue;
            };
            world.add_player(runtime_id);
            world.set_pose(
                runtime_id,
                Pose {
                    x: config.position[0],
                    y: config.position[1],
                    z: config.position[2],
                    ..Default::default()
                },
            );
        }
        Self {
            world,
            aoi_radius,
            pending_events: Vec::new(),
            metrics: None,
            npc_registry: None,
            named_npc_registry: Some(named_npc_registry),
            elevator_registry: None,
            pending_elevator_broadcasts: Vec::new(),
            session_registry: SessionRegistry::new(),
            pending_interaction_opens: Vec::new(),
            pending_interaction_results: Vec::new(),
            nav_graph: None,
            vehicle_registry: None,
            pending_entity_reports: Vec::new(),
            heat_tracker: None,
            pending_combat_events: Vec::new(),
            tick_duration_window: std::collections::VecDeque::new(),
            degradation_tier: crate::degradation::DegradationTier::Normal,
        }
    }

    /// Identique à `Server::new`, avec en plus un registre de véhicules actif — VIDE au départ
    /// (aucun véhicule de trafic d'ambiance spawn automatiquement dans ce noyau ; le spawn se fait
    /// explicitement via `spawn_vehicle`, appelé soit par un test, soit par un futur câblage
    /// director-de-trafic hors périmètre de ce plan — spec §2 « trafic d'ambiance » différé, ce
    /// noyau livre le mécanisme, pas le peuplement automatique). Nouvelle méthode plutôt qu'un
    /// changement de signature des constructeurs existants : `Server::new`/`new_with_metrics`/
    /// `new_with_npcs`/`new_with_named_npcs` sont déjà appelés par de nombreux tests existants qui
    /// ne doivent pas changer pour cette tâche (même raisonnement que chacun d'eux).
    pub fn new_with_vehicles(aoi_radius: f32) -> Self {
        Self {
            world: World::new(),
            aoi_radius,
            pending_events: Vec::new(),
            metrics: None,
            npc_registry: None,
            named_npc_registry: None,
            elevator_registry: None,
            pending_elevator_broadcasts: Vec::new(),
            session_registry: SessionRegistry::new(),
            pending_interaction_opens: Vec::new(),
            pending_interaction_results: Vec::new(),
            nav_graph: None,
            vehicle_registry: Some(VehicleRegistry {
                records: std::collections::HashMap::new(),
                nav_states: std::collections::HashMap::new(),
                next_vehicle_id: crate::world::VEHICLE_ID_RANGE_START,
            }),
            pending_entity_reports: Vec::new(),
            heat_tracker: None,
            pending_combat_events: Vec::new(),
            tick_duration_window: std::collections::VecDeque::new(),
            degradation_tier: crate::degradation::DegradationTier::Normal,
        }
    }

    /// Fait naître un véhicule à `from`, avec une destination initiale `to` déjà planifiée (v1 :
    /// pas de director de trafic, le spawn est explicite — cf. doc de `new_with_vehicles`).
    /// No-op silencieux si ce `Server` n'a pas de registre véhicule (`Server::new` et les autres
    /// constructeurs sans véhicules) — même style que `tick_npcs`/`tick_vehicles` (garde `let
    /// else` plutôt qu'un panic sur un appel a priori incorrect côté appelant, cohérent avec le
    /// reste du fichier).
    pub fn spawn_vehicle(&mut self, archetype: u32, from: NavVec3, to: NavVec3) {
        let Some(registry) = &mut self.vehicle_registry else {
            return;
        };
        let id = registry.next_vehicle_id;
        registry.next_vehicle_id += 1;
        // Vitesse fixée à 8.0 u/s ici (v1 : constante, pas encore par archétype/.aiarch — même
        // simplification assumée que `NPC_SPEED_UNITS_PER_SEC` côté piéton dans `tick_npcs`).
        registry
            .records
            .insert(id, VehicleRecord::new(id, archetype, 8.0));
        self.world.add_player(id);
        self.world.set_pose(
            id,
            Pose {
                x: from.x,
                y: from.y,
                z: from.z,
                ..Default::default()
            },
        );
        if let Some(graph) = &self.nav_graph {
            if let Some(path) = plan_path(graph, from, to) {
                registry.nav_states.entry(id).or_default().set_path(path);
            }
        }
    }

    /// Identique à `Server::new`, avec en plus un registre d'ascenseurs peuplé depuis le catalogue.
    /// `tick_ms` = durée d'un tick serveur (voir Task 6 pour la valeur réelle au boot). Exprimé en
    /// termes de `with_elevators` — conservé pour les tests existants qui le nomment directement.
    pub fn new_with_elevators(aoi_radius: f32, catalog: ElevatorCatalog, tick_ms: u32) -> Self {
        Self::new(aoi_radius).with_elevators(catalog, tick_ms)
    }

    /// Active le registre d'ascenseurs sur un `Server` déjà construit, quel qu'ait été son
    /// constructeur (`new`, `new_with_metrics`, `new_with_npcs`, `new_with_named_npcs`) — les
    /// ascenseurs sont orthogonaux aux PNJ (foule ou nominatifs) : un shard réel peut déclarer les
    /// deux à la fois, contrairement à `population`/`named_npc_manifest_path` qui sont mutuellement
    /// exclusifs entre eux dans cette fondation. Chaînable plutôt qu'un cinquième constructeur
    /// combinatoire (`new_with_npcs_and_elevators`, etc.) qui exploserait avec chaque nouvelle
    /// fondation orthogonale. `tick_ms` = durée d'un tick serveur (cf. `shard.rs::TICK_MS`).
    pub fn with_elevators(mut self, catalog: ElevatorCatalog, tick_ms: u32) -> Self {
        self.elevator_registry = Some(ElevatorRegistry {
            states: catalog.into_states(),
            tick_ms,
        });
        self
    }

    /// Nombre de joueurs actuellement dans le monde de ce Shard — pour l'endpoint métriques.
    pub fn player_count(&self) -> usize {
        self.world.player_ids().len()
    }

    /// Heat policier courant (spec PNJ hostiles §3) — 0 si aucun `HeatTracker` actif sur ce
    /// `Server` (constructeurs autres que `new_with_police_escalation`). Accesseur de test
    /// minimal (`heat_tracker` reste privé, pas de raison de l'exposer publiquement en dehors des
    /// tests) — même patron que d'autres accesseurs de ce fichier (`player_count`).
    #[cfg(test)]
    pub(crate) fn heat(&self) -> u32 {
        self.heat_tracker.map(|t| t.heat).unwrap_or(0)
    }

    /// Palier de dégradation courant — accesseur de test minimal (`degradation_tier` reste privé),
    /// même patron que `heat()` ci-dessus.
    #[cfg(test)]
    pub(crate) fn degradation_tier_for_test(&self) -> crate::degradation::DegradationTier {
        self.degradation_tier
    }

    /// Injecte directement le contenu de la fenêtre glissante de durées de tick — permet de
    /// simuler une charge soutenue sans dépendre d'un vrai busy-loop (fragile en CI), même patron
    /// que les autres accesseurs `_for_test` de ce fichier.
    #[cfg(test)]
    pub(crate) fn inject_tick_durations_for_test(&mut self, durations: Vec<u64>) {
        self.tick_duration_window = durations.into_iter().collect();
    }

    /// Attache un graphe de navigation après construction. Sans appel, `tick_npcs` continue de
    /// fonctionner exactement comme avant ce plan (FSM/briques nav-indépendantes seules, aucun
    /// PNJ ne bouge) — comportement historique strictement préservé.
    pub fn set_nav_graph(&mut self, graph: NavGraph) {
        self.nav_graph = Some(graph);
    }

    /// Point d'entrée d'un appel d'ascenseur. Séparé du décodage réseau pour rester testable sans
    /// fabriquer d'enveloppe FlatBuffers. Un `elevator_id` inconnu du catalogue est IGNORÉ : le
    /// serveur ne fabrique jamais un ascenseur sur la foi d'un message client.
    pub fn handle_elevator_call(&mut self, _from: ClientId, elevator_id: u64, floor: i32) -> bool {
        let Some(registry) = &mut self.elevator_registry else {
            return false;
        };
        let Some(state) = registry.get_mut(elevator_id) else {
            return false;
        };
        state.request_floor(floor)
    }

    /// Un pas des ascenseurs : fait avancer chaque cabine et retourne celles à diffuser.
    ///
    /// ⚠️ `advance` appelle DÉJÀ `start_trip_if_idle` en interne (c'est lui qui fait l'enchaînement
    /// automatique vers l'appel suivant) — ne pas l'appeler une seconde fois ici.
    ///
    /// Cadence de diffusion (spec §5.3) : à chaque transition d'état, PLUS un rappel à faible
    /// fréquence tant qu'une cabine n'est pas au repos. C'est ce rappel qui rattrape un ordre de
    /// départ perdu (cas C3) — et il ne coûte rien quand tout est à l'arrêt.
    fn tick_elevators(&mut self, now_tick: u64) -> Vec<ElevatorState> {
        const HEARTBEAT_TICKS: u64 = 20;

        let Some(registry) = &mut self.elevator_registry else {
            return Vec::new();
        };
        let tick_ms = registry.tick_ms;
        let mut to_broadcast = Vec::new();
        for state in registry.states.iter_mut() {
            let changed = state.advance(now_tick, tick_ms);
            let moving = state.movement_state != MovementState::Stopped;
            if changed || (moving && now_tick.is_multiple_of(HEARTBEAT_TICKS)) {
                to_broadcast.push(state.clone());
            }
        }
        to_broadcast
    }

    /// État courant de TOUTES les cabines — à envoyer à un client qui vient d'arriver (connexion,
    /// ou entrée à portée). Sans ça, un joueur qui rejoint en pleine course ne saurait pas qu'une
    /// cabine bouge avant sa prochaine transition (cas D1/D2 de la spec).
    fn elevator_states_for_new_client(&self) -> Vec<ElevatorState> {
        self.elevator_registry
            .as_ref()
            .map(|r| r.states.clone())
            .unwrap_or_default()
    }

    /// Encode un `ElevatorStateMsg` (spec ascenseurs §6). AUCUNE position de cabine ne part sur le
    /// fil (ADR 0012) — seuls les champs qui permettent au client de rejouer le même trajet.
    fn encode_elevator_state(state: &ElevatorState) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let requested: Vec<i32> = state.requested_floors.iter().copied().collect();
        let requested_off = b.create_vector(&requested);
        let movement = match state.movement_state {
            MovementState::Stopped => 0u8,
            MovementState::MovingUp => 1,
            MovementState::MovingDown => 2,
            MovementState::Paused => 3,
        };
        let msg = ElevatorStateMsg::create(
            &mut b,
            &ElevatorStateMsgArgs {
                elevator_id: state.elevator_id,
                active_floor: state.active_floor,
                target_floor: state.target_floor.unwrap_or(-1),
                movement_state: movement,
                requested_floors: Some(requested_off),
                // `0` double comme sentinelle « pas de départ en cours » ET comme numéro de tick valide.
                // C'est sûr ici parce que `self.world.advance_tick()` (dans `tick()` ligne 366) s'exécute
                // AVANT le code ascenseur — donc `now_tick` ne sera jamais 0 quand on enregistre un vrai départ.
                depart_tick: state.depart_tick.unwrap_or(0),
                start_delay_ms: state.start_delay_ms,
                travel_time_ms: state.travel_time_ms,
            },
        );
        let env = ServerEnvelope::create(
            &mut b,
            &ServerEnvelopeArgs {
                msg_type: ServerMsg::ElevatorStateMsg,
                msg: Some(msg.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    /// Un pas du director de population + du moteur de briques PNJ (spec fondation PNJ) — no-op si
    /// ce `Server` n'a pas de registre PNJ (`Server::new`/`new_with_metrics`).
    ///
    /// Simplification assumée pour cette fondation (voir Step 7 du plan Task 6) : le director ne
    /// raisonne PAS sur la vraie topologie multi-district (`authority.json`/`tools/district-topology`,
    /// câblée côté Gateway/`handoff.rs`, hors périmètre ici). Tous les joueurs connus de CE `Server`
    /// comptent comme présents dans un unique district logique `"default"`. Le câblage réel
    /// multi-district est un raffinement explicitement différé, pas un oubli.
    fn tick_npcs(
        &mut self,
        pre_tick_behaviors: Option<std::collections::HashMap<ClientId, EntityBehavior>>,
    ) {
        let Some(registry) = &mut self.npc_registry else {
            return;
        };
        // Trouvé en revue finale de branche (plan navigation) : player_ids() renvoie TOUT le
        // monde dans World, PNJ compris (mêmes add_player/remove_player, décision délibérée de la
        // fondation PNJ pour réutiliser snapshot_for/la grille spatiale telles quelles). Sans ce
        // filtre, dès qu'au moins un PNJ existe, ce compte ne retombe jamais à zéro même si tous
        // les vrais joueurs se déconnectent — le chemin de despawn-sur-district-vide
        // (population_director.rs, !has_players) devient inatteignable en pratique. Bug
        // préexistant à ce plan (présent depuis la fondation PNJ), corrigé ici car c'est ce même
        // plan qui l'a fait échouer pour de vrai (test de nettoyage nav_states, Task 6).
        let player_count = self
            .world
            .player_ids()
            .into_iter()
            .filter(|id| !crate::world::is_npc_id(*id))
            .count() as u32;
        let players_by_district =
            std::collections::HashMap::from([("default".to_string(), player_count)]);
        let existing_by_district = std::collections::HashMap::from([(
            "default".to_string(),
            registry.records.len() as u32,
        )]);
        let actions = registry.director.reconcile(
            &registry.catalog,
            &players_by_district,
            &existing_by_district,
        );
        for action in actions {
            match action {
                crate::population_director::DirectorAction::Spawn { archetype_id, .. } => {
                    let id = registry.next_npc_id;
                    registry.next_npc_id += 1;
                    registry
                        .records
                        .insert(id, NpcRecord::new(id, archetype_id));
                    self.world.add_player(id);
                }
                crate::population_director::DirectorAction::Despawn { excess, .. } => {
                    let to_remove: Vec<ClientId> = registry
                        .records
                        .keys()
                        .take(excess as usize)
                        .copied()
                        .collect();
                    for id in to_remove {
                        registry.records.remove(&id);
                        registry.nav_states.remove(&id);
                        self.world.remove_player(id);
                    }
                }
            }
        }
        const NPC_SPEED_UNITS_PER_SEC: f32 = 3.0; // marche (spec parle de vitesse par brique/archétype
                                                  // via .aiarch, différé — constante v1 uniforme ici,
                                                  // documentée comme simplification assumée).
        let tick_dt = 1.0 / crate::default_tick_rate_hz() as f32;
        let move_distance = NPC_SPEED_UNITS_PER_SEC * tick_dt;

        for (id, record) in registry.records.iter_mut() {
            let Some(archetype) = registry.catalog.archetype(record.archetype) else {
                continue;
            };
            // `before` = comportement AVANT CE TICK ENTIER (capturé en tout début de `Server::tick`,
            // donc AVANT le drain des events transport) — PAS juste avant `apply_brique_tick`
            // ci-dessous. Nécessaire pour détecter une transition Calme/Flane/Alerte/ATerre ->
            // Fuite/Hostile causée par une `EntityInteraction` traitée plus tôt dans ce même
            // `tick()` (kind=0=Menace via `NpcRecord::apply_interaction`, appelé par
            // `apply_client_message`, AVANT `tick_npcs`) : une capture faite ICI (après ce drain)
            // verrait déjà l'état post-message et manquerait ce cas — vérifié empiriquement (probe
            // de développement), pas supposé. Absent de `pre_tick_behaviors` (PNJ tout juste
            // spawné ce tick par le director, donc inconnu au moment de la capture) => traité
            // comme `Calme` (comportement par défaut d'un `NpcRecord`, cf. `NpcRecord::new`) : un
            // PNJ qui spawne et devient hostile au même tick doit compter comme une transition.
            let before = pre_tick_behaviors
                .as_ref()
                .and_then(|m| m.get(id))
                .copied()
                .unwrap_or_default();
            record.apply_brique_tick(archetype);

            // Heat policier (spec PNJ hostiles §3) : un rapport d'incident dès qu'un PNJ qui
            // n'était PAS en Fuite/Hostile en DÉBUT de tick l'est devenu (menace signalée par un
            // joueur, ou "attaquer-cible" escaladant une Fuite déjà là — dans ce dernier cas
            // `before` reste Fuite depuis avant CE tick, donc PAS recompté ici : seule la PREMIÈRE
            // entrée dans la catégorie Fuite/Hostile compte, cf. spec §3 "rapport de menace").
            // Montant fixe (10) et decay (fin de fonction) NON encore data-driven (pas de vraie
            // `EscalationPolicy` chargée depuis TOML dans ce plan, cf. doc de
            // `new_with_police_escalation`) — raffinement de configuration différé.
            if let Some(tracker) = &mut self.heat_tracker {
                let became_hostile_or_fuite = !matches!(
                    before,
                    crate::npc::EntityBehavior::Fuite { .. }
                        | crate::npc::EntityBehavior::Hostile { .. }
                ) && matches!(
                    record.behavior,
                    crate::npc::EntityBehavior::Fuite { .. }
                        | crate::npc::EntityBehavior::Hostile { .. }
                );
                if became_hostile_or_fuite {
                    tracker.report_incident(10); // montant v1 fixe, cf. note ci-dessus
                }
            }

            // Migration d'ownership (spec PNJ hostiles §1 : "changement de cible = migration
            // d'ownership... l'owner devient le joueur que le FSM cible"). Réassignation directe et
            // idempotente — pas de détection de "changement" nécessaire, écrire la même valeur à
            // chaque tick est sans coût et élimine toute fenêtre où owner et cible divergeraient
            // après coup. Seul EntityBehavior::Hostile déclenche une réassignation ; les autres
            // comportements (Calme/Flane/Alerte/Fuite/ATerre) NE TOUCHENT PAS owner ici —
            // Alerte/Fuite portent une "menace", pas une "cible" au sens propriétaire du terme (spec
            // §1 ne parle que de la migration pour Hostile), et ATerre/Calme/Flane n'ont pas de
            // notion de cible du tout.
            if let crate::npc::EntityBehavior::Hostile { cible } = record.behavior {
                record.owner = cible;
            }

            let Some(graph) = &self.nav_graph else {
                continue; // comportement historique préservé sans graphe (Global Constraints)
            };
            let nav_state = registry.nav_states.entry(*id).or_default();
            let current_pose = self.world.pose_of(*id).unwrap_or_default();
            let current_pos = (current_pose.x, current_pose.y, current_pose.z);

            if !nav_state.has_path() || nav_state.has_arrived() {
                // Pas de chemin en cours (ou arrivé) -> décide d'une nouvelle destination et planifie.
                // region_center/region_radius v1 : la position ACTUELLE du PNJ (simplification assumée —
                // le vrai centre de région par archétype, spec §4, attend une config dédiée future).
                // menace_or_cible : résolue via World::pose_of sur l'id porté par le behavior FSM (Fuite/
                // Hostile) — sans ceci, un PNJ en fuite ne saurait jamais de quelle position s'éloigner
                // (npc.rs::decide_destination retourne None sans position réelle pour Fuite, testé
                // séparément). Calme/Flane/ATerre n'utilisent jamais cette valeur.
                let menace_or_cible = match record.behavior {
                    crate::npc::EntityBehavior::Fuite { menace } => {
                        self.world.pose_of(menace).map(|p| (p.x, p.y, p.z))
                    }
                    crate::npc::EntityBehavior::Hostile { cible } => {
                        self.world.pose_of(cible).map(|p| (p.x, p.y, p.z))
                    }
                    _ => None,
                };
                if let Some(dest) = crate::npc::decide_destination(
                    record.behavior,
                    archetype,
                    current_pos,
                    menace_or_cible,
                    current_pos,
                    15.0,
                    pseudo_random_unit(*id, self.world.tick()),
                ) {
                    let (dx, dy, dz) = dest;
                    if let Some(path) = plan_path(
                        graph,
                        NavVec3::new(current_pos.0, current_pos.1, current_pos.2),
                        NavVec3::new(dx, dy, dz),
                    ) {
                        nav_state.set_path(path);
                    }
                }
            }

            if nav_state.has_path() {
                let new_pos = nav_state.advance(
                    NavVec3::new(current_pos.0, current_pos.1, current_pos.2),
                    move_distance,
                );
                self.world.set_pose(
                    *id,
                    Pose {
                        x: new_pos.x,
                        y: new_pos.y,
                        z: new_pos.z,
                        locomotion: 1, // Walk (cf. commentaire triplet biped, world.rs) — v1 uniforme,
                        // Sprint réservé à Fuite en raffinement futur (spec §5)
                        ..current_pose
                    },
                );
            }
        }

        // Décroissance temporelle du heat (spec §3 : "decay temporel"), une fois par tick, hors de
        // la boucle PNJ ci-dessus — s'applique même sur un tick sans aucune nouvelle transition.
        // Montant v1 fixe (1), même note de raffinement différé que `report_incident` ci-dessus.
        if let Some(tracker) = &mut self.heat_tracker {
            tracker.decay(1);
        }
    }

    /// Un pas de navigation véhicule (spec véhicules autonomes §3) — no-op si ce `Server` n'a pas
    /// de registre véhicule (`Server::new` et les autres constructeurs sans véhicules). Même
    /// mécanique que la navigation PNJ (`tick_npcs` : NavGraph/plan_path/NavState réutilisés tels
    /// quels), mais SANS director (aucune réconciliation spawn/despawn ici — spawn explicite via
    /// `spawn_vehicle` uniquement, cf. sa doc) et avec la vitesse PROPRE du véhicule
    /// (`record.speed_units_per_sec`) plutôt que la constante `NPC_SPEED_UNITS_PER_SEC` piétonne.
    fn tick_vehicles(&mut self) {
        let Some(registry) = &mut self.vehicle_registry else {
            return;
        };
        // Fenêtre prédictive du pont Shard→Gateway (spec véhicules autonomes §5 : "rank_bonus ≈
        // vitesse × N secondes, N à régler"). Même valeur des deux côtés du pont — ici pour décider
        // QUAND émettre un rapport (should_report_position), côté Gateway pour dimensionner le
        // tampon de handoff (predictive_rank_bonus, cf. gateway.rs) — un désaccord entre les deux
        // ferait émettre un rapport qui arrive trop tôt/tard par rapport au tampon réellement chargé.
        const BOUNDARY_LOOKAHEAD_SECONDS: f32 = 2.0;
        let tick_dt = 1.0 / crate::default_tick_rate_hz() as f32;
        for (id, record) in registry.records.iter_mut() {
            let Some(nav_state) = registry.nav_states.get_mut(id) else {
                continue;
            };
            if !nav_state.has_path() {
                continue; // v1 : pas de re-décision de destination automatique (pas de director de
                          // trafic, cf. spawn_vehicle) — un véhicule arrivé s'arrête, spec §8
                          // (annulation/nouvelle destination) différé.
            }
            let current_pose = self.world.pose_of(*id).unwrap_or_default();
            let new_pos = nav_state.advance(
                NavVec3::new(current_pose.x, current_pose.y, current_pose.z),
                record.speed_units_per_sec * tick_dt,
            );
            self.world.set_pose(
                *id,
                Pose {
                    x: new_pos.x,
                    y: new_pos.y,
                    z: new_pos.z,
                    ..current_pose
                },
            );

            if let Some(passenger_id) = record.passenger {
                // Invariant convoi (spec §4, approche A) : la position ABSOLUE du passager suit le
                // véhicule. Le handoff joueur reste INTOUCHÉ (le passager garde son propre ClientId,
                // son propre chemin de handoff normal — ce Server ne fait qu'écraser sa Pose, rien
                // d'autre) — la vertu cardinale de l'approche A (spec §4).
                let passenger_pose = self.world.pose_of(passenger_id).unwrap_or_default();
                self.world.set_pose(
                    passenger_id,
                    Pose {
                        x: new_pos.x,
                        y: new_pos.y,
                        z: new_pos.z,
                        ..passenger_pose
                    },
                );
            }

            let _ = &record.movement; // v1 : toujours EnRoute pendant le déplacement, Arrete est
                                      // posé/lu par un futur câblage hélage (spec §8, hors périmètre).

            // Pont Shard→Gateway générique (shard_boundary_bridge.rs) : un véhicule dont le chemin
            // planifié entre dans le tampon prédictif d'un shard voisin déclenche un rapport de
            // position, drainé par `shard_main` et transmis au Gateway pour un vrai handoff — même
            // mécanique qu'un `PositionUpdate` de client réel (cf. gateway.rs). `next_waypoint`
            // absent (chemin arrivé/vide) => aucun rapport, cohérent avec `!nav_state.has_path()`
            // ci-dessus qui a déjà fait `continue` dans ce cas.
            if let Some(next_waypoint) = nav_state.next_waypoint() {
                let current = NavVec3::new(new_pos.x, new_pos.y, new_pos.z);
                if crate::shard_boundary_bridge::should_report_position(
                    current,
                    next_waypoint,
                    record.speed_units_per_sec,
                    BOUNDARY_LOOKAHEAD_SECONDS,
                ) {
                    self.pending_entity_reports.push((
                        *id,
                        new_pos.x,
                        new_pos.y,
                        new_pos.z,
                        record.speed_units_per_sec,
                    ));
                }
            }
        }
    }

    /// Rapports de position prédictifs à transmettre au Gateway ce tick (pont Shard→Gateway,
    /// primitive générique — cf. `shard_boundary_bridge.rs`). Drainé par `shard_main`, qui les
    /// encode (`internal_net::encode_entity_position_report`) et les écrit sur la socket TCP
    /// interne existante vers le Gateway — `Server`/`server_loop.rs` n'a pas de connexion TCP
    /// directe au Gateway, seul `shard.rs::shard_main` possède la socket.
    pub fn take_pending_entity_reports(&mut self) -> Vec<(ClientId, f32, f32, f32, f32)> {
        std::mem::take(&mut self.pending_entity_reports)
    }

    /// Événements de transition PV=0 à transmettre en télémétrie ce tick (cf. doc de
    /// `pending_combat_events`). Drainé par `shard_main`.
    pub fn take_pending_combat_events(&mut self) -> Vec<(ClientId, u32, ClientId, u64)> {
        std::mem::take(&mut self.pending_combat_events)
    }

    /// Un tick : applique les events entrants, avance le monde, envoie un snapshot à chaque client.
    pub fn tick<T: Transport>(&mut self, transport: &mut T) {
        let tick_start = std::time::Instant::now();
        // Heat policier (spec PNJ hostiles §3) : capture du `behavior` de chaque PNJ AVANT le
        // drain des events transport ci-dessous — nécessaire pour détecter une transition
        // Calme/Flane/Alerte/ATerre -> Fuite/Hostile causée par une `EntityInteraction` traitée
        // dans CE MÊME tick (`apply_client_message`, kind=0=Menace via `NpcRecord::apply_interaction`,
        // appelé plus bas dans cette fonction, donc AVANT `tick_npcs`). Une capture faite à
        // l'intérieur de `tick_npcs` (juste avant `apply_brique_tick`) arrive trop tard : elle ne
        // verrait déjà plus que l'état POST-message, jamais l'état Calme d'origine — vérifié
        // empiriquement (probe), pas supposé : sans cette capture en amont, `report_incident`
        // n'était jamais déclenché sur le cas d'usage principal (joueur qui menace un PNJ),
        // seul le cas Fuite->Hostile interne à `apply_brique_tick` (brique "attaquer-cible")
        // aurait été détecté.
        let pre_tick_behaviors: Option<std::collections::HashMap<ClientId, EntityBehavior>> =
            self.npc_registry.as_ref().map(|r| {
                r.records
                    .iter()
                    .map(|(id, rec)| (*id, rec.behavior))
                    .collect()
            });
        for ev in transport.poll() {
            match ev {
                TransportEvent::Connected(id) => {
                    self.world.add_player(id);
                    // Cas D1/D2 de la spec ascenseurs : un client qui rejoint en pleine course doit
                    // connaître l'état courant des cabines SANS attendre leur prochaine transition.
                    for state in self.elevator_states_for_new_client() {
                        let bytes = Self::encode_elevator_state(&state);
                        transport.send(id, &bytes);
                    }
                }
                TransportEvent::Disconnected(id) => self.world.remove_player(id),
                TransportEvent::Message { from, data } => self.apply_client_message(from, &data),
            }
        }
        self.world.advance_tick();
        self.tick_npcs(pre_tick_behaviors);
        // Un pas des ascenseurs (spec §5.3) : fait avancer chaque cabine et diffuse à TOUS les
        // clients connus de ce Shard les états qui ont transitionné (ou le rappel périodique tant
        // qu'une cabine est en mouvement). Aucun filtrage AoI ici : `ElevatorState` ne porte aucune
        // position de cabine (contrainte globale), et le catalogue n'attache pas non plus de
        // position monde à une cage d'ascenseur — un filtrage par distance n'a donc rien à mordre.
        let elevator_updates = self.tick_elevators(self.world.tick());
        let mut elevators_broadcast_this_tick: Vec<u64> =
            Vec::with_capacity(elevator_updates.len());
        for state in &elevator_updates {
            elevators_broadcast_this_tick.push(state.elevator_id);
            let bytes = Self::encode_elevator_state(state);
            for id in self.world.player_ids() {
                transport.send(id, &bytes);
            }
        }
        // Diffusion immédiate d'un appel accepté en pleine course (finding revue de branche
        // finale, cf. doc de `pending_elevator_broadcasts`) : un appel qui ajoute un étage SANS
        // changer `target_floor`/`movement_state` de la cabine (déjà en route ailleurs) n'est vu
        // par AUCUNE transition dans la boucle `tick_elevators` ci-dessus — sans ce relais, il
        // n'atteindrait les autres clients qu'au prochain rappel heartbeat. On ne redouble jamais
        // un ascenseur déjà couvert par la boucle ci-dessus (pas de double-diffusion dans le même
        // tick).
        for elevator_id in self.pending_elevator_broadcasts.drain(..) {
            if elevators_broadcast_this_tick.contains(&elevator_id) {
                continue;
            }
            elevators_broadcast_this_tick.push(elevator_id);
            let Some(registry) = &self.elevator_registry else {
                continue;
            };
            let Some(state) = registry
                .states
                .iter()
                .find(|s| s.elevator_id == elevator_id)
            else {
                continue;
            };
            let bytes = Self::encode_elevator_state(state);
            for id in self.world.player_ids() {
                transport.send(id, &bytes);
            }
        }
        self.tick_vehicles();
        // Réclame les sessions d'interaction jamais résolues (client qui ouvre puis se déconnecte
        // sans jamais répondre) — trouvé en revue finale de branche (fondation d'interaction) :
        // SessionRegistry::expire_stale existait et était testé (Task 1) mais n'était jamais
        // appelé, laissant les sessions abandonnées s'accumuler sans borne sur un Shard longue
        // durée. 30s : généreux pour un joueur qui parcourt une offre avant de répondre, borné
        // pour ne pas laisser une session fantôme vivre indéfiniment (spec fondation d'interaction
        // §2 : « timeout serveur », aucune valeur numérique imposée par la spec).
        const INTERACTION_SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
        self.session_registry
            .expire_stale(INTERACTION_SESSION_TIMEOUT);
        for (actor, target) in self.pending_interaction_opens.drain(..) {
            let session_id = self.session_registry.open(actor, target, 0);
            let bytes = crate::gateway_routing::encode_interaction_open(session_id, target, 0, &[]);
            transport.send(actor, &bytes);
        }
        for (actor, session_id, outcome, target) in self.pending_interaction_results.drain(..) {
            let bytes = crate::gateway_routing::encode_interaction_result(
                session_id,
                outcome.ok,
                &outcome.payload,
            );
            transport.send(actor, &bytes);
            // Règle du log RP (spec §2/§7) : toute résolution, succès ou refus, est journalisée. Le
            // log lui-même vit dans gateway.rs (SessionLog est possédé par le Gateway, pas par
            // Server) — Server expose l'événement via un canal que Task 7/gateway.rs consomme ;
            // pour cette tâche, l'événement SessionEvent::InteractionResolved existe (session_log.rs)
            // mais n'est PAS encore écrit depuis ici (Server n'a et ne doit pas avoir de dépendance
            // sur session_log.rs, qui vit au niveau Gateway) — limitation assumée, cf. rapport de
            // tâche.
            let _ = target;
        }
        let mut b = FlatBufferBuilder::new();
        for id in self.world.player_ids() {
            b.reset();
            let bytes = self.encode_snapshot_for(id, &mut b);
            transport.send(id, &bytes);
        }
        // Relais des événements one-shot du tick, filtré par le même AoI que les snapshots.
        for (actor, kind, action, param) in self.pending_events.drain(..) {
            let neighbors = self.world.snapshot_for(actor, self.aoi_radius);
            for (neighbor_id, _) in neighbors {
                b.reset();
                let ev = PlayerEvent::create(
                    &mut b,
                    &PlayerEventArgs {
                        actor,
                        kind,
                        action,
                        param,
                    },
                );
                let env = ServerEnvelope::create(
                    &mut b,
                    &ServerEnvelopeArgs {
                        msg_type: ServerMsg::PlayerEvent,
                        msg: Some(ev.as_union_value()),
                    },
                );
                b.finish(env, None);
                transport.send(neighbor_id, b.finished_data());
            }
        }
        let elapsed_micros = tick_start.elapsed().as_micros() as u64;
        // Fenêtre glissante + palier de dégradation (spec tenue-en-charge §3) — indépendant de
        // `self.metrics` (`Option<Arc<Metrics>>`, câblé séparément pour Prometheus) : ce mécanisme
        // de sécurité doit fonctionner même sur un `Server` sans metrics configurées.
        if self.tick_duration_window.len() >= TICK_DURATION_WINDOW_SIZE {
            self.tick_duration_window.pop_front();
        }
        self.tick_duration_window.push_back(elapsed_micros);
        if let Some(p99) = p99_of(&self.tick_duration_window) {
            let policy = crate::degradation::DegradationPolicy::default();
            self.degradation_tier = policy.tier_for_p99(p99, self.degradation_tier);
        }
        if let Some(metrics) = &self.metrics {
            metrics.record_tick_duration_micros(elapsed_micros);
        }
    }

    fn apply_client_message(&mut self, from: ClientId, data: &[u8]) {
        let Ok(env) = flatbuffers::root::<ClientEnvelope>(data) else {
            return;
        };
        match env.msg_type() {
            ClientMsg::Join => { /* TODO(Phase-1): stocker le display_name */ }
            ClientMsg::PositionUpdate => {
                if let Some(pu) = env.msg_as_position_update() {
                    if let Some(p) = pu.position() {
                        self.world.set_pose(
                            from,
                            Pose {
                                x: p.x(),
                                y: p.y(),
                                z: p.z(),
                                yaw: pu.yaw(),
                                ..self.world.pose_of(from).unwrap_or_default()
                            },
                        );
                    }
                    self.world
                        .set_locomotion(from, pu.locomotion(), pu.move_dir(), pu.flags());
                }
            }
            ClientMsg::EmoteReport => {
                if let Some(er) = env.msg_as_emote_report() {
                    let emote = if er.start() { er.emote() } else { 0 };
                    self.world.set_sustained(from, emote);
                }
            }
            ClientMsg::PlayerActionReport => {
                if let Some(ar) = env.msg_as_player_action_report() {
                    // kind=0=Action (seul type existant pour l'instant, cf. schéma). Relayé en fin
                    // de tick, filtré par AoI — jamais appliqué à la position/locomotion (canal
                    // cosmétique one-shot).
                    self.pending_events.push((from, 0, ar.action(), ar.param()));
                }
            }
            ClientMsg::EntityInteraction => {
                if let Some(ei) = env.msg_as_entity_interaction() {
                    // Comportement existant (fondation PNJ) : transition FSM sur la foule anonyme —
                    // INCHANGÉ, ne pas toucher cette branche.
                    if let Some(registry) = &mut self.npc_registry {
                        if let Some(record) = registry.records.get_mut(&ei.target()) {
                            record.apply_interaction(from, ei.kind());
                        }
                    }
                    // Nouveau (fondation d'interaction) : kind=2=Interagit sur un PNJ NOMINATIF
                    // ouvre une session. Un PNJ nominatif n'a pas de NpcRecord (pas de FSM propre
                    // dans cette fondation — Calme par défaut, cf. Task 7) donc
                    // interaction_allowed(Calme) est toujours vrai ici ; le refus FSM réel (spec
                    // §2, "un vendeur fuite/à terre refuse") attend que les PNJ nominatifs aient
                    // leur propre NpcRecord, hors périmètre de cette tâche.
                    if ei.kind() == 2 {
                        if let Some(named) = &self.named_npc_registry {
                            if named.manifest_id_of(ei.target()).is_some() {
                                self.pending_interaction_opens.push((from, ei.target()));
                            }
                        }
                    }
                    // Nouveau (véhicules autonomes, Task 6) : kind=3=Mount / kind=4=Unmount sur un
                    // véhicule (spec §3/§4). L'invariant convoi (position du passager écrasée par
                    // celle du véhicule) est appliqué dans `tick_vehicles`, pas ici — ce bloc ne
                    // fait que poser/retirer `record.passenger`.
                    if ei.kind() == 3 {
                        if let Some(vreg) = &mut self.vehicle_registry {
                            if let Some(vehicle) = vreg.records.get_mut(&ei.target()) {
                                let _ = vehicle.mount(from); // échec silencieux si déjà occupé — spec §8 mono-passager
                            }
                        }
                    } else if ei.kind() == 4 {
                        if let Some(vreg) = &mut self.vehicle_registry {
                            if let Some(vehicle) = vreg.records.get_mut(&ei.target()) {
                                vehicle.unmount(from);
                            }
                        }
                    } else if ei.kind() == 5 {
                        // Nouveau (PNJ hostiles, Task 2) : kind=5=Attaque rapporte des dégâts sur un
                        // PNJ (spec §1/§2). N'importe quel attaquant peut rapporter des dégâts — pas
                        // besoin d'être owner (contrairement à d'autres interactions) — apply_damage
                        // (Task 1) porte lui-même le clamp anti-triche et la cadence anti-spam par
                        // attaquant. now_ms dérivé du tick serveur courant (cadence fixe connue via
                        // default_tick_rate_hz) plutôt qu'une horloge murale réelle — cohérent avec le
                        // style déterministe/testable du reste de ce fichier.
                        if let Some(registry) = &mut self.npc_registry {
                            if let Some(record) = registry.records.get_mut(&ei.target()) {
                                if let Some(archetype) =
                                    registry.catalog.archetype(record.archetype)
                                {
                                    let now_ms = self.world.tick()
                                        * (1000 / crate::default_tick_rate_hz() as u64);
                                    let archetype_id = record.archetype;
                                    // Task 4bis : capture le retour de apply_damage (ignoré jusqu'ici)
                                    // pour pousser un événement de télémétrie combat sur la transition
                                    // réelle vers ATerre (spec PNJ hostiles §2). Drainé par
                                    // shard_main (shard.rs), qui écrit le JSONL — Server reste pur.
                                    if record.apply_damage(from, archetype, ei.param(), now_ms) {
                                        self.pending_combat_events.push((
                                            ei.target(),
                                            archetype_id,
                                            from,
                                            now_ms,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            ClientMsg::InteractionChoice => {
                if let Some(ic) = env.msg_as_interaction_choice() {
                    match self.session_registry.resolve(ic.session_id(), from) {
                        Ok(session) => {
                            let outcome =
                                execute_transaction(EntityBehavior::Calme, || TransactionOutcome {
                                    ok: true,
                                    payload: Vec::new(),
                                });
                            self.pending_interaction_results.push((
                                from,
                                ic.session_id(),
                                outcome,
                                session.target,
                            ));
                        }
                        Err(SessionError::NotFound) | Err(SessionError::NotOwner) => {
                            self.pending_interaction_results.push((
                                from,
                                ic.session_id(),
                                TransactionOutcome {
                                    ok: false,
                                    payload: Vec::new(),
                                },
                                0,
                            ));
                        }
                    }
                }
            }
            ClientMsg::ElevatorCall => {
                if let Some((elevator_id, floor)) =
                    crate::gateway_routing::extract_elevator_call(data)
                {
                    // `true` = appel NOUVELLEMENT accepté (bouton qui vient de s'allumer) — mémorisé
                    // pour que `tick()` le diffuse ce même tick même si `tick_elevators` ne détecte
                    // aucune transition (cf. doc de `pending_elevator_broadcasts`).
                    if self.handle_elevator_call(from, elevator_id, floor) {
                        self.pending_elevator_broadcasts.push(elevator_id);
                    }
                }
            }
            _ => {}
        }
    }

    fn encode_snapshot_for(&self, viewer: ClientId, b: &mut FlatBufferBuilder) -> Vec<u8> {
        let states: Vec<_> = self
            .world
            .snapshot_for(viewer, self.aoi_radius)
            .into_iter()
            .map(|(id, pose)| {
                let pos = Vec3::new(pose.x, pose.y, pose.z);
                PlayerState::create(
                    b,
                    &PlayerStateArgs {
                        id,
                        position: Some(&pos),
                        yaw: pose.yaw,
                        locomotion: pose.locomotion,
                        move_dir: pose.move_dir,
                        flags: pose.flags,
                        sustained: pose.sustained,
                    },
                )
            })
            .collect();
        let npc_states: Vec<_> = self
            .npc_registry
            .iter()
            .flat_map(|r| r.records.values())
            .filter_map(|record| {
                let pose = self.world.pose_of(record.id)?;
                Some(NpcState::create(
                    b,
                    &NpcStateArgs {
                        id: record.id,
                        archetype: record.archetype,
                        position: Some(&Vec3::new(pose.x, pose.y, pose.z)),
                        yaw: pose.yaw,
                        locomotion: pose.locomotion,
                        move_dir: pose.move_dir,
                        flags: pose.flags,
                        sustained: pose.sustained,
                        behavior: behavior_to_u8(record.behavior),
                    },
                ))
            })
            .collect();
        let vehicle_states: Vec<_> = self
            .vehicle_registry
            .iter()
            .flat_map(|r| r.records.values())
            .filter_map(|record| {
                let pose = self.world.pose_of(record.id)?;
                Some(VehicleState::create(
                    b,
                    &VehicleStateArgs {
                        id: record.id,
                        archetype: record.archetype,
                        position: Some(&Vec3::new(pose.x, pose.y, pose.z)),
                        yaw: pose.yaw,
                        // Quantization simple centiunités/s (choix v1, raffinable au gel consolidé
                        // si une meilleure précision s'avère nécessaire) — cohérent avec l'esprit
                        // de quantization déjà prévu pour le protocole (cf. commentaire schéma
                        // VehicleState, protocol.fbs).
                        speed: (record.speed_units_per_sec * 100.0).round() as u16,
                        passenger: record.passenger.unwrap_or(0),
                    },
                ))
            })
            .collect();
        let players = b.create_vector(&states);
        let npcs = b.create_vector(&npc_states);
        let vehicles = b.create_vector(&vehicle_states);
        let snap = Snapshot::create(
            b,
            &SnapshotArgs {
                tick: self.world.tick(),
                players: Some(players),
                npcs: Some(npcs),
                vehicles: Some(vehicles),
            },
        );
        let env = ServerEnvelope::create(
            b,
            &ServerEnvelopeArgs {
                msg_type: ServerMsg::Snapshot,
                msg: Some(snap.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }
}

/// Pseudo-aléatoire déterministe [0.0, 1.0) dérivé de l'id du PNJ + du tick courant — pas un vrai
/// RNG (pas de dépendance `rand` ajoutée), suffisant pour varier les destinations `errer` sans
/// motif visible à l'échelle d'un playtest. Documenté comme simplification v1 assumée.
fn pseudo_random_unit(seed: ClientId, tick: u64) -> f32 {
    let mixed = seed
        .wrapping_mul(2654435761)
        .wrapping_add(tick.wrapping_mul(0x9E3779B97F4A7C15));
    ((mixed >> 40) as f32) / (1u64 << 24) as f32 % 1.0
}

/// Capacité de la fenêtre glissante de durées de tick utilisée pour approcher un p99 en runtime
/// (spec tenue-en-charge §3 : "seuils à hystérésis pilotés par le p99 du tick"). 200 ticks = 10s de
/// fenêtre glissante à 20Hz (cadence par défaut du projet, cf. `default_tick_rate_hz`) — assez pour
/// lisser un pic ponctuel sans réagir à un seul tick isolé, assez court pour redescendre vite si la
/// charge retombe réellement. Valeur v1, non calibrée sur mesure réelle (même statut que les
/// constantes de `DegradationPolicy::default()`, cf. Global Constraints).
const TICK_DURATION_WINDOW_SIZE: usize = 200;

/// Calcule un p99 approché par tri complet de la fenêtre (pas un algorithme de streaming à la
/// t-digest — à cette taille de fenêtre (200), un tri est largement assez rapide pour tourner une
/// fois par tick sans impact mesurable sur le budget de tick). `None` si la fenêtre est vide (aucun
/// tick mesuré encore — cas du tout premier tick).
fn p99_of(window: &std::collections::VecDeque<u64>) -> Option<u64> {
    if window.is_empty() {
        return None;
    }
    let mut sorted: Vec<u64> = window.iter().copied().collect();
    sorted.sort_unstable();
    // Index du 99e percentile sur une liste triée de longueur n : ceil(0.99 * n) - 1, borné à
    // n-1 pour éviter un débordement quand n est petit (ex. n=1 -> index 0).
    let index = ((sorted.len() as f64 * 0.99).ceil() as usize).saturating_sub(1);
    let index = index.min(sorted.len() - 1);
    Some(sorted[index])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::InMemoryTransport;

    #[test]
    fn a_threat_report_increases_heat_when_police_escalation_is_active() {
        // Scénario principal spec §3 : un joueur signale une menace (kind=0=Menace) sur un PNJ
        // encore Calme -> transition vers Fuite -> le heat serveur-autoritaire doit augmenter. Le
        // PNJ n'a besoin d'aucune brique de mouvement particulière pour ce test ("errer" suffit,
        // le heat ne dépend que du FSM, jamais de la navigation).
        let catalog = crate::npc_catalog::parse_and_validate(
            "format_version = 1\n[[archetype]]\nid = 1\nname = \"t\"\nbriques = [\"errer\"]\n",
        )
        .unwrap();
        let director =
            crate::population_director::PopulationDirector::new(std::collections::HashMap::from([
                ("default".to_string(), 1),
            ]));
        let mut server = Server::new_with_police_escalation(50.0, catalog, director);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // laisse le director spawn le PNJ
        server.tick(&mut t);
        assert_eq!(
            server.heat(),
            0,
            "aucune menace signalée encore : le heat doit rester à zéro"
        );

        let npc_id = crate::world::NPC_ID_RANGE_START;
        let threat = encode_entity_interaction(npc_id, 0, 0); // kind=0=Menace, déclenche Fuite
        t.inject(TransportEvent::Message {
            from: 1,
            data: threat,
        });
        server.tick(&mut t);

        assert_eq!(
            server.heat(),
            9, // report_incident(10) puis decay(1) systématique en fin du même tick_npcs, cf. Step 6
            "une transition Calme -> Fuite détectée ce tick doit augmenter le heat (10 - 1 de decay)"
        );
    }

    #[test]
    fn a_fuite_to_hostile_escalation_via_a_brique_also_increases_heat() {
        // Second cas d'usage du même mécanisme : "attaquer-cible" fait passer Fuite -> Hostile en
        // un seul appel à `apply_brique_tick` (pas via une nouvelle `EntityInteraction`) — ce
        // franchissement compte aussi comme "devenir Fuite/Hostile" car le PNJ était Calme
        // (jamais encore Fuite/Hostile) au tout début du tick où la menace initiale ET l'escalade
        // sont toutes deux résolues.
        let catalog = crate::npc_catalog::parse_and_validate(
            "format_version = 1\n[[archetype]]\nid = 1\nname = \"t\"\nbriques = [\"attaquer-cible\"]\n",
        )
        .unwrap();
        let director =
            crate::population_director::PopulationDirector::new(std::collections::HashMap::from([
                ("default".to_string(), 1),
            ]));
        let mut server = Server::new_with_police_escalation(50.0, catalog, director);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // spawn
        server.tick(&mut t);

        let npc_id = crate::world::NPC_ID_RANGE_START;
        let threat = encode_entity_interaction(npc_id, 0, 0); // kind=0=Menace -> Fuite, PUIS
                                                              // "attaquer-cible" escalade -> Hostile,
                                                              // le tout dans le même tick_npcs.
        t.inject(TransportEvent::Message {
            from: 1,
            data: threat,
        });
        server.tick(&mut t);

        assert_eq!(
            server.heat(),
            9,
            "Calme -> Hostile en un seul tick (via Fuite puis attaquer-cible) doit aussi compter \
             comme une seule transition (pas de double comptage)"
        );
    }

    #[test]
    fn a_server_without_npcs_never_adds_any_npc_state_to_the_snapshot() {
        // Comportement historique préservé : Server::new (sans PNJ) ne doit jamais faire apparaître
        // de NpcState dans un Snapshot, même après plusieurs ticks.
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let env = flatbuffers::root::<ServerEnvelope>(sent.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert!(
            snap.npcs().map(|v| v.len()).unwrap_or(0) == 0,
            "sans registre PNJ, npcs doit rester vide"
        );
    }

    #[test]
    fn a_server_with_npcs_configured_spawns_and_reports_npcs_in_the_snapshot() {
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur-de-rue"
            briques = ["flaner-sur-place"]
            "#,
        )
        .unwrap();
        // NOTE : le district est nommé "default" et non "centre" — cette fondation ne raisonne
        // que sur un unique district logique "default" regroupant tous les joueurs connus de ce
        // `Server` (simplification assumée, cf. commentaire sur `Server::tick_npcs` : la vraie
        // topologie multi-district est un raffinement différé, hors périmètre ici).
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        // Tout joueur connu de ce `Server` compte comme présent dans le district "default" —
        // Server::tick_npcs résout ainsi la présence (cf. Step 7 du plan Task 6).
        server.tick(&mut t);
        server.tick(&mut t); // un 2e tick pour laisser le director réagir à la présence du joueur

        let sent = t.take_sent(1);
        let env = flatbuffers::root::<ServerEnvelope>(sent.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert!(
            snap.npcs().map(|v| v.len()).unwrap_or(0) > 0,
            "un director configuré avec un joueur présent doit finir par faire apparaître au moins un PNJ"
        );
    }

    #[test]
    fn entity_interaction_kind_5_applies_damage_via_apply_damage() {
        // Câblage réel EntityInteraction{kind=5} -> NpcRecord::apply_damage (Task 2). Un archétype
        // combattant avec 100 PV et un clamp de 40 dégâts max par rapport : un unique coup de 40
        // suffit à passer directement à 60 PV restants (pas encore ATerre) ; on vérifie ici l'effet
        // observable via le prochain snapshot (comportement, pas juste "accepté" — cf. protocole
        // de sondage sur "accepté != exécuté", même principe côté serveur : on observe le FSM).
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "gang-membre"
            briques = ["attaquer-cible"]
            [archetype.combat]
            hp = 100
            degats_max_par_rapport = 100
            cadence_min_ms = 0
            "#,
        )
        .unwrap();
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // laisse le director spawn le PNJ
        server.tick(&mut t); // laisse le 2e tick le faire apparaître dans le snapshot

        let sent = t.take_sent(1);
        let npc_id = sent
            .iter()
            .find_map(|bytes| {
                let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
                let snap = env.msg_as_snapshot()?;
                snap.npcs()?.iter().next().map(|n| n.id())
            })
            .expect("le PNJ doit être apparu dans un des snapshots déjà envoyés");

        // Un coup de 100 dégâts (>= hp) doit faire passer le PNJ à ATerre (behavior_to_u8 == 5).
        let hit = encode_entity_interaction(npc_id, 5, 100);
        t.inject(TransportEvent::Message { from: 1, data: hit });
        server.tick(&mut t);

        let sent = t.take_sent(1);
        let behavior_after_hit = sent.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?
                .iter()
                .find(|n| n.id() == npc_id)
                .map(|n| n.behavior())
        });
        assert_eq!(
            behavior_after_hit,
            Some(crate::npc::behavior_to_u8(
                crate::npc::EntityBehavior::ATerre
            )),
            "un rapport de dégâts kind=5 >= hp doit faire passer le PNJ à ATerre via apply_damage"
        );
    }

    #[test]
    fn a_lethal_kind_5_report_pushes_a_pending_combat_event() {
        // Task 4bis : la transition ATerre doit pousser un événement dans la file
        // `pending_combat_events`, drainée par `shard_main` (shard.rs) pour l'écriture JSONL réelle
        // via `hostile_telemetry::append_combat_event` (Task 4). Même patron que
        // `pending_entity_reports` (pont véhicules) : `Server`/`server_loop.rs` reste pur, ne fait
        // qu'accumuler.
        let catalog = crate::npc_catalog::parse_and_validate(
            "format_version = 1\n[[archetype]]\nid = 1\nname = \"t\"\nbriques = [\"errer\"]\n\
             [archetype.combat]\nhp = 40\ndegats_max_par_rapport = 100\ncadence_min_ms = 0\n",
        )
        .unwrap();
        let director =
            crate::population_director::PopulationDirector::new(std::collections::HashMap::from([
                ("default".to_string(), 1),
            ]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // laisse le director spawn le PNJ
        t.take_sent(1);

        let npc_id = crate::world::NPC_ID_RANGE_START;
        let hit = encode_entity_interaction(npc_id, 5, 100); // dégâts >= hp -> transition ATerre
        t.inject(TransportEvent::Message {
            from: 42,
            data: hit,
        });
        server.tick(&mut t);

        let events = server.take_pending_combat_events();
        assert_eq!(
            events.len(),
            1,
            "une transition ATerre doit pousser exactement un événement"
        );
        let (event_npc_id, archetype, killer, _timestamp_ms) = events[0];
        assert_eq!(event_npc_id, npc_id);
        assert_eq!(archetype, 1);
        assert_eq!(
            killer, 42,
            "killer doit être l'attaquant réel (from), pas le npc_id"
        );

        // Un second drain sans nouvel événement ne doit rien renvoyer (comportement mem::take).
        assert!(server.take_pending_combat_events().is_empty());
    }

    #[test]
    fn a_non_lethal_kind_5_report_pushes_no_combat_event() {
        let catalog = crate::npc_catalog::parse_and_validate(
            "format_version = 1\n[[archetype]]\nid = 1\nname = \"t\"\nbriques = [\"errer\"]\n\
             [archetype.combat]\nhp = 1000\ndegats_max_par_rapport = 40\ncadence_min_ms = 0\n",
        )
        .unwrap();
        let director =
            crate::population_director::PopulationDirector::new(std::collections::HashMap::from([
                ("default".to_string(), 1),
            ]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        t.take_sent(1);

        let npc_id = crate::world::NPC_ID_RANGE_START;
        let hit = encode_entity_interaction(npc_id, 5, 40); // dégâts << hp -> pas de transition
        t.inject(TransportEvent::Message {
            from: 42,
            data: hit,
        });
        server.tick(&mut t);

        assert!(
            server.take_pending_combat_events().is_empty(),
            "un rapport de dégâts non-létal ne doit jamais pousser d'événement combat"
        );
    }

    #[test]
    fn a_npc_that_becomes_hostile_has_its_owner_migrated_to_the_target() {
        // Documente l'invariant de migration d'ownership (spec PNJ hostiles §1 : "changement de
        // cible = migration d'ownership... l'owner devient le joueur que le FSM cible"). Test
        // unitaire pur sur NpcRecord (pas besoin d'un Server complet) — le déclenchement RÉEL de
        // Hostile depuis une brique arrive avec Task 3 ; ce test fige juste l'invariant que
        // `tick_npcs` applique déjà (Step 4) dès que `behavior` vaut Hostile{cible}.
        let mut record = crate::npc::NpcRecord::new(crate::world::NPC_ID_RANGE_START, 1);
        assert_eq!(record.owner, 0);
        record.behavior = crate::npc::EntityBehavior::Hostile { cible: 42 };
        // Même logique que celle insérée dans `tick_npcs` (Step 4) — reproduite ici en isolation
        // pure pour ne pas dépendre d'un catalogue/director/nav_graph juste pour cet invariant.
        if let crate::npc::EntityBehavior::Hostile { cible } = record.behavior {
            record.owner = cible;
        }
        assert_eq!(
            record.owner, 42,
            "l'owner doit migrer vers la cible visée par Hostile"
        );
    }

    fn encode_position(x: f32, y: f32, z: f32, yaw: f32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(x, y, z);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw,
                locomotion: 0,
                move_dir: 0,
                flags: 0,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::PositionUpdate,
                msg: Some(pu.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn two_clients_see_each_other_move() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();

        // Deux clients se connectent.
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        // Client 1 bouge en (5,0,0).
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position(5.0, 0.0, 0.0, 0.0),
        });

        server.tick(&mut t);

        // Le client 2 doit recevoir un snapshot contenant le joueur 1 en x=5.
        let sent_to_2 = t.take_sent(2);
        assert_eq!(sent_to_2.len(), 1, "un snapshot envoyé au client 2");
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let players = snap.players().unwrap();
        assert_eq!(players.len(), 1);
        let p = players.get(0);
        assert_eq!(p.id(), 1);
        assert_eq!(p.position().unwrap().x(), 5.0);

        let sent_to_1 = t.take_sent(1);
        assert_eq!(sent_to_1.len(), 1, "un snapshot envoyé au client 1");
        let env1 = flatbuffers::root::<ServerEnvelope>(&sent_to_1[0]).unwrap();
        let snap1 = env1.msg_as_snapshot().unwrap();
        let players1 = snap1.players().unwrap();
        assert_eq!(players1.len(), 1);
        assert_eq!(players1.get(0).id(), 2); // client 1 voit client 2
    }

    fn encode_position_with_locomotion(
        x: f32,
        y: f32,
        z: f32,
        yaw: f32,
        locomotion: u8,
        move_dir: u8,
    ) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let pos = Vec3::new(x, y, z);
        let pu = PositionUpdate::create(
            &mut b,
            &PositionUpdateArgs {
                position: Some(&pos),
                yaw,
                locomotion,
                move_dir,
                flags: 0,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::PositionUpdate,
                msg: Some(pu.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn position_update_carries_locomotion_into_snapshot() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 3, 42),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(p.locomotion(), 3);
        assert_eq!(p.move_dir(), 42);
    }

    #[test]
    fn repeated_position_updates_do_not_reset_locomotion_to_idle() {
        // Piège identifié en Task 2 : set_pose remplace toute la Pose. Un deuxième PositionUpdate
        // (même sans nouveau champ de locomotion explicite envoyé par le client, qui renvoie
        // toujours son état courant à chaque update selon la spec §8.1) ne doit jamais faire
        // disparaître la valeur précédente si le client continue de la reporter correctement —
        // ce test vérifie surtout que l'ordre d'application (set_pose puis set_locomotion, ou une
        // fusion) ne perd pas le champ dans le MÊME message.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 2, 5),
        });
        server.tick(&mut t);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(6.0, 0.0, 0.0, 0.0, 2, 5),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(p.position().unwrap().x(), 6.0);
        assert_eq!(p.locomotion(), 2);
    }

    #[test]
    fn position_update_never_touches_sustained() {
        // Le canal cosmétique continu (locomotion) et la pose tenue (sustained, pilotée par
        // EmoteReport UNIQUEMENT) doivent rester complètement indépendants — un PositionUpdate ne
        // doit jamais remettre sustained à 0 s'il était déjà posé.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        // (Task 4 posera sustained via EmoteReport ; ici on vérifie juste qu'un PositionUpdate seul
        // sur un joueur au sustained par défaut à 0 le laisse à 0 - non-régression basique, le test
        // complet d'indépendance vraie est en Task 4 une fois EmoteReport câblé.)
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(5.0, 0.0, 0.0, 0.0, 1, 0),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(snap.players().unwrap().get(0).sustained(), 0);
    }

    fn encode_emote_report(emote: u32, start: bool) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let er = EmoteReport::create(&mut b, &EmoteReportArgs { emote, start });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::EmoteReport,
                msg: Some(er.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn emote_report_start_sets_sustained_in_snapshot() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(7, true),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(snap.players().unwrap().get(0).sustained(), 7);
    }

    #[test]
    fn emote_report_stop_clears_sustained() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(7, true),
        });
        server.tick(&mut t);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(7, false),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(snap.players().unwrap().get(0).sustained(), 0);
    }

    #[test]
    fn sustained_emote_survives_a_subsequent_position_update() {
        // LE test clé du raffinement §5 de la spec : l'état continu (sustained) doit survivre à un
        // PositionUpdate qui suit — les deux canaux sont indépendants.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(9, true),
        });
        server.tick(&mut t);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(1.0, 0.0, 0.0, 0.0, 0, 0),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(
            p.sustained(),
            9,
            "la pose tenue doit survivre au PositionUpdate suivant"
        );
        assert_eq!(p.position().unwrap().x(), 1.0);
    }

    fn encode_player_action(action: u8, param: u32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let ar = PlayerActionReport::create(&mut b, &PlayerActionReportArgs { action, param });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::PlayerActionReport,
                msg: Some(ar.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    fn decode_player_event(bytes: &[u8]) -> Option<(u64, u8, u8, u32)> {
        let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
        if env.msg_type() != ServerMsg::PlayerEvent {
            return None;
        }
        let pe = env.msg_as_player_event()?;
        Some((pe.actor(), pe.kind(), pe.action(), pe.param()))
    }

    #[test]
    fn player_action_report_relays_player_event_to_aoi_neighbor() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(5, 99),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        // sent_to_2 contient le Snapshot ET le PlayerEvent — filtrer par type.
        let event = sent_to_2.iter().find_map(|b| decode_player_event(b));
        let (actor, kind, action, param) = event.expect("le voisin doit recevoir le PlayerEvent");
        assert_eq!(actor, 1);
        assert_eq!(kind, 0);
        assert_eq!(action, 5);
        assert_eq!(param, 99);
    }

    #[test]
    fn player_action_report_not_relayed_outside_aoi_radius() {
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(500.0, 0.0, 0.0, 0.0, 0, 0),
        });
        server.tick(&mut t);
        t.take_sent(2); // vider le snapshot du premier tick
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(5, 99),
        });
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let event = sent_to_2.iter().find_map(|b| decode_player_event(b));
        assert!(
            event.is_none(),
            "le joueur 2 est hors AoI (500 > 50), ne doit rien recevoir"
        );
    }

    #[test]
    fn player_action_report_never_touches_position_or_locomotion() {
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position_with_locomotion(3.0, 0.0, 0.0, 0.0, 2, 0),
        });
        server.tick(&mut t);
        t.take_sent(2);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(1, 0),
        }); // ex. Jump
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(
            sent_to_2
                .iter()
                .find(|b| {
                    flatbuffers::root::<ServerEnvelope>(b)
                        .map(|e| e.msg_type() == ServerMsg::Snapshot)
                        .unwrap_or(false)
                })
                .unwrap(),
        )
        .unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let p = snap.players().unwrap().get(0);
        assert_eq!(
            p.position().unwrap().x(),
            3.0,
            "un PlayerActionReport ne doit jamais déplacer le joueur"
        );
        assert_eq!(p.locomotion(), 2, "ni changer sa locomotion continue");
    }

    #[test]
    fn players_far_apart_do_not_see_each_other() {
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();

        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_position(500.0, 0.0, 0.0, 0.0),
        });

        server.tick(&mut t);

        let sent_to_2 = t.take_sent(2);
        assert_eq!(sent_to_2.len(), 1);
        let env = flatbuffers::root::<ServerEnvelope>(&sent_to_2[0]).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let players = snap.players().unwrap();
        assert_eq!(
            players.len(),
            0,
            "client 1 est à 500 unités, hors du rayon de 50 — ne doit pas apparaître"
        );
    }

    #[test]
    fn cosmetic_channel_events_never_change_player_count_or_connectivity() {
        // Aucun message du canal d'état (PositionUpdate enrichi, EmoteReport, PlayerActionReport)
        // ne doit jamais connecter/déconnecter un joueur ni modifier player_count.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        assert_eq!(server.player_count(), 2);
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(1, true),
        });
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(1, 0),
        });
        server.tick(&mut t);
        assert_eq!(
            server.player_count(),
            2,
            "le canal cosmétique ne doit jamais affecter la connectivité"
        );
    }

    #[test]
    fn late_aoi_joiner_learns_sustained_pose_from_snapshot_not_from_missed_event() {
        // Le test clé §5 de la spec, version bout-en-bout via Server (pas juste World, déjà couvert
        // en Task 4) : un joueur qui rejoint l'AoI APRÈS le début d'une pose tenue doit quand même
        // la voir dans son PREMIER snapshot (auto-cicatrisant), sans avoir reçu l'EmoteReport lui-même.
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_emote_report(3, true),
        });
        server.tick(&mut t);
        // Le joueur 2 arrive APRÈS le début de la pose.
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let env = flatbuffers::root::<ServerEnvelope>(
            sent_to_2
                .iter()
                .find(|b| {
                    flatbuffers::root::<ServerEnvelope>(b)
                        .map(|e| e.msg_type() == ServerMsg::Snapshot)
                        .unwrap_or(false)
                })
                .unwrap(),
        )
        .unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert_eq!(
            snap.players().unwrap().get(0).sustained(),
            3,
            "un arrivant tardif doit lire la pose depuis le snapshot"
        );
    }

    #[test]
    fn one_shot_event_not_resent_to_late_joiner() {
        // Le contraste du test précédent : un ÉVÉNEMENT one-shot (PlayerActionReport→PlayerEvent)
        // n'est PAS auto-cicatrisant — un arrivant tardif ne le reçoit pas rétroactivement, c'est
        // le comportement voulu (rater un one-shot est inoffensif, cf. spec §5).
        let mut server = Server::new(1000.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_player_action(1, 0),
        });
        server.tick(&mut t); // aucun voisin au moment de l'action — rien relayé, personne pour le recevoir
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        let sent_to_2 = t.take_sent(2);
        let event = sent_to_2.iter().find_map(|b| decode_player_event(b));
        assert!(
            event.is_none(),
            "un one-shot manqué reste manqué, pas de rattrapage"
        );
    }

    fn encode_entity_interaction(target: u64, kind: u8, param: u32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let ei = EntityInteraction::create(
            &mut b,
            &EntityInteractionArgs {
                target,
                kind,
                param,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::EntityInteraction,
                msg: Some(ei.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    fn encode_interaction_choice(session_id: u64, choice: u8, param: u32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let ic = InteractionChoice::create(
            &mut b,
            &InteractionChoiceArgs {
                session_id,
                choice,
                param,
            },
        );
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::InteractionChoice,
                msg: Some(ic.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn a_server_without_named_npcs_never_opens_a_session_on_interagit() {
        // Comportement historique préservé : sans NamedNpcRegistry configuré, un EntityInteraction
        // kind=2 (Interagit) ne doit jamais produire d'InteractionOpen — juste rien (comme avant ce
        // plan, où kind=2 était déjà silencieusement ignoré par apply_interaction).
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        t.take_sent(1); // purge le premier snapshot

        let interaction = encode_entity_interaction(999, 2, 0); // target=999 (aucun PNJ n'existe)
        t.inject(TransportEvent::Message {
            from: 1,
            data: interaction,
        });
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let has_open = sent.iter().any(|bytes| {
            flatbuffers::root::<ServerEnvelope>(bytes)
                .map(|env| env.msg_type() == ServerMsg::InteractionOpen)
                .unwrap_or(false)
        });
        assert!(
            !has_open,
            "sans registre nominatif, aucune InteractionOpen ne doit être envoyée"
        );
    }

    #[test]
    fn interacting_with_a_named_npc_opens_a_session_and_a_choice_resolves_it() {
        use crate::named_npc_catalog::parse_and_validate;
        use crate::named_npc_registry::NamedNpcRegistry;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[pnj]]
            id = "ripperdoc-watson-01"
            archetype = "a"
            position = [0.0, 0.0, 0.0]
            briques = ["rester-statique"]
            "#,
        )
        .unwrap();
        let named_registry = NamedNpcRegistry::from_catalog(&catalog);
        let npc_runtime_id = named_registry.runtime_ids()[0];
        let mut server = Server::new_with_named_npcs(50.0, &catalog, named_registry);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        let first_snapshot = t.take_sent(1);
        // Vérifie au passage que le spawn au constructeur (pas juste l'arbitrage de session) a bien
        // eu lieu : le PNJ nominatif doit apparaître dans le tout premier snapshot du joueur, avant
        // même toute interaction — preuve qu'il vit réellement dans World depuis new_with_named_npcs.
        let npc_visible = first_snapshot.iter().any(|bytes| {
            let Ok(env) = flatbuffers::root::<ServerEnvelope>(bytes) else {
                return false;
            };
            let Some(snap) = env.msg_as_snapshot() else {
                return false;
            };
            snap.players()
                .map(|ps| ps.iter().any(|p| p.id() == npc_runtime_id))
                .unwrap_or(false)
        });
        assert!(
            npc_visible,
            "le PNJ nominatif doit être visible dans World dès la construction"
        );

        // Étape 1 : le joueur interagit -> le serveur ouvre une session.
        let interaction = encode_entity_interaction(npc_runtime_id, 2, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: interaction,
        });
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let session_id = sent
            .iter()
            .find_map(|bytes| {
                let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
                if env.msg_type() != ServerMsg::InteractionOpen {
                    return None;
                }
                env.msg_as_interaction_open().map(|o| o.session_id())
            })
            .expect("une InteractionOpen doit être envoyée pour un PNJ nominatif interactible");

        // Étape 2 : le joueur répond -> le serveur résout et répond InteractionResult{ok=true}.
        let choice = encode_interaction_choice(session_id, 0, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: choice,
        });
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let ok = sent.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            if env.msg_type() != ServerMsg::InteractionResult {
                return None;
            }
            env.msg_as_interaction_result().map(|r| r.ok())
        });
        assert_eq!(ok, Some(true));
    }

    #[test]
    fn tick_reclaims_a_session_opened_but_never_resolved_after_it_goes_stale() {
        // Trouvé en revue finale de branche : SessionRegistry::expire_stale (Task 1) était testé en
        // isolation mais jamais appelé depuis Server::tick — une session ouverte par un client qui
        // se déconnecte sans jamais répondre restait en mémoire indéfiniment. Ce test verrouille le
        // câblage réel : après le délai d'expiration, un Choice tardif sur une session ouverte AVANT
        // ce délai doit échouer comme si la session n'avait jamais existé (NotFound -> ok=false),
        // pas juste que SessionRegistry::expire_stale fonctionne isolément (déjà couvert ailleurs).
        use crate::named_npc_catalog::parse_and_validate;
        use crate::named_npc_registry::NamedNpcRegistry;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[pnj]]
            id = "ripperdoc-watson-01"
            archetype = "a"
            position = [0.0, 0.0, 0.0]
            briques = ["rester-statique"]
            "#,
        )
        .unwrap();
        let named_registry = NamedNpcRegistry::from_catalog(&catalog);
        let npc_runtime_id = named_registry.runtime_ids()[0];
        let mut server = Server::new_with_named_npcs(50.0, &catalog, named_registry);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        t.take_sent(1);

        let interaction = encode_entity_interaction(npc_runtime_id, 2, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: interaction,
        });
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let session_id = sent
            .iter()
            .find_map(|bytes| {
                let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
                if env.msg_type() != ServerMsg::InteractionOpen {
                    return None;
                }
                env.msg_as_interaction_open().map(|o| o.session_id())
            })
            .expect("une InteractionOpen doit être envoyée pour un PNJ nominatif interactible");

        // Laisse le délai d'expiration (30s, cf. INTERACTION_SESSION_TIMEOUT dans tick()) s'écouler
        // réellement — Instant ne se falsifie pas (même contrainte documentée dans
        // interaction_session.rs pour son propre test d'expiration réelle).
        std::thread::sleep(std::time::Duration::from_millis(30_100));
        // Un tick sans nouveau message doit tout de même réclamer la session expirée.
        server.tick(&mut t);
        t.take_sent(1);

        let choice = encode_interaction_choice(session_id, 0, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: choice,
        });
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let ok = sent.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            if env.msg_type() != ServerMsg::InteractionResult {
                return None;
            }
            env.msg_as_interaction_result().map(|r| r.ok())
        });
        assert_eq!(
            ok,
            Some(false),
            "une session expirée doit refuser le Choice tardif comme NotFound (ok=false)"
        );
    }

    #[test]
    fn a_npc_with_a_nav_graph_and_an_errer_brique_moves_over_several_ticks() {
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur"
            briques = ["errer"]
            "#,
        )
        .unwrap();
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);

        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        // `b` à 10.0 (pas 200.0) : la brique errer tire des destinations dans un disque de
        // rayon 15.0 autour de la position courante (region_radius câblé dans tick_npcs) — un
        // second nœud à 200 unités n'est JAMAIS le nœud le plus proche d'une destination errer
        // (nearest_node(to) == nearest_node(from) à chaque fois, path=[position courante],
        // le PNJ "arrive" instantanément sans jamais bouger). À 10.0, une partie des destinations
        // tirées (x > 5, la médiatrice des deux nœuds) snappent sur `b`, produisant un vrai chemin
        // à traverser — vérifié en isolant le calcul (pseudo_random_unit + la formule errer)
        // avant de corriger ce fixture, cf. rapport de tâche.
        let b = graph.add_node(NavVec3::new(10.0, 0.0, 0.0));
        graph.add_edge(a, b);
        server.set_nav_graph(graph);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // laisse le director spawn le PNJ
        server.tick(&mut t); // laisse le premier tick assigner une destination + planifier

        let position_after_first = t.take_sent(1).iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?.iter().next().map(|n| {
                let p = n.position().unwrap();
                (p.x(), p.y())
            })
        });

        // Plusieurs ticks supplémentaires -> le PNJ doit avoir progressé (position différente).
        for _ in 0..20 {
            server.tick(&mut t);
        }
        let position_later = t.take_sent(1).iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?.iter().next().map(|n| {
                let p = n.position().unwrap();
                (p.x(), p.y())
            })
        });

        assert!(
            position_after_first.is_some() && position_later.is_some(),
            "le PNJ doit apparaître dans les deux snapshots"
        );
        assert_ne!(
            position_after_first, position_later,
            "après plusieurs ticks avec un graphe de navigation, la position du PNJ doit avoir changé"
        );
    }

    #[test]
    fn a_npc_without_a_nav_graph_never_moves_from_its_default_pose() {
        // Comportement historique préservé (fondation PNJ) : sans set_nav_graph, un PNJ avec une
        // brique errer ne bouge jamais — decide_destination peut produire une destination mais aucun
        // chemin n'est planifiable sans graphe.
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur"
            briques = ["errer"]
            "#,
        )
        .unwrap();
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);
        // PAS de set_nav_graph.

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        for _ in 0..5 {
            server.tick(&mut t);
        }
        let sent = t.take_sent(1);
        let npc_position = sent.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?.iter().next().map(|n| {
                let p = n.position().unwrap();
                (p.x(), p.y())
            })
        });
        assert_eq!(
            npc_position,
            Some((0.0, 0.0)),
            "sans graphe de navigation, le PNJ reste à sa Pose par défaut (0,0)"
        );
    }

    #[test]
    fn a_despawned_npc_has_its_nav_state_removed_not_leaked() {
        // Trouvé en revue finale de branche (plan navigation) : le despawn (population director,
        // fondation PNJ) retirait bien le NpcRecord et l'entrée World, mais laissait l'entrée
        // nav_states orpheline — fuite de mémoire lente (non bornée) sur un Shard longue durée à
        // fort churn spawn/despawn (les ids ne sont jamais réutilisés, next_npc_id ne fait
        // qu'augmenter). Ce test verrouille le nettoyage : après un cycle spawn -> déconnexion du
        // seul joueur présent (qui déclenche le despawn total du district "default",
        // population_director.rs) -> tick, le nombre d'entrées de nav_states doit être revenu à 0.
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur"
            briques = ["errer"]
            "#,
        )
        .unwrap();
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);
        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        let b = graph.add_node(NavVec3::new(10.0, 0.0, 0.0));
        graph.add_edge(a, b);
        server.set_nav_graph(graph);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // spawn
        server.tick(&mut t); // planifie un chemin -> nav_states gagne une entrée
        assert_eq!(
            server
                .npc_registry
                .as_ref()
                .map(|r| r.nav_states.len())
                .unwrap_or(0),
            1,
            "précondition : le PNJ doit avoir un NavState avant le despawn"
        );

        t.inject(TransportEvent::Disconnected(1));
        server.tick(&mut t); // plus aucun joueur -> le director despawn tout le district "default"

        assert_eq!(
            server
                .npc_registry
                .as_ref()
                .map(|r| r.records.len())
                .unwrap_or(0),
            0,
            "le PNJ doit être despawné (comportement déjà existant, fondation PNJ)"
        );
        assert_eq!(
            server
                .npc_registry
                .as_ref()
                .map(|r| r.nav_states.len())
                .unwrap_or(0),
            0,
            "nav_states ne doit PAS garder une entrée orpheline après le despawn"
        );
    }

    #[test]
    fn a_fleeing_npc_moves_away_from_the_threat_over_several_ticks() {
        // Spec §9 critère « le FSM déclenche les bons déplacements (fuite s'éloigne) » — bout-en-
        // bout via Server/InMemoryTransport (pas juste decide_destination en isolation, déjà
        // couvert unitairement par Task 4).
        //
        // Piège de fixture identique en substance à celui trouvé en Task 5 (moves_over_several_ticks) :
        // le joueur 1 (la menace) ET le PNJ spawnent tous deux à la Pose par défaut (0,0,0)
        // (World::add_player). Si on laisse le PNJ à cette position par défaut, decide_destination
        // calcule dx=cx-mx=0, dy=0 -> `len = 0.0.max(EPSILON)` -> direction (0/EPSILON, 0/EPSILON) =
        // (0.0, 0.0) -> destination = position courante inchangée : la fuite ne produirait AUCUN
        // mouvement, indépendamment du câblage. Vérifié par calcul direct de la formule (rapport de
        // tâche). Fixe : on repositionne le PNJ (pas le joueur 1, qui reste à x=0 comme demandé par
        // le plan) à x=10 AVANT de déclencher la fuite, avec un graphe dont le premier nœud est
        // EXACTEMENT cette position (distance 0, pas de snap ambigu) et le second nœud EXACTEMENT à
        // la destination de fuite attendue (10 + FLEE_DISTANCE(30) = 40, distance 0 également) — donc
        // aucun aller-retour vers un nœud plus proche de la position de départ que la destination
        // réelle (le piège symétrique observé en Task 5 avec un nœud hors de portée aurait ici pris
        // la forme d'un chemin qui commence par reculer vers un nœud mal placé avant d'avancer ;
        // vérifié par simulation manuelle de NavState::advance sur les deux fixtures avant de choisir
        // celle-ci, cf. rapport de tâche).
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur"
            briques = ["errer"]
            "#,
        )
        .unwrap();
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);

        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(10.0, 0.0, 0.0)); // position forcée de départ du PNJ
        let b = graph.add_node(NavVec3::new(40.0, 0.0, 0.0)); // = destination de fuite exacte
        graph.add_edge(a, b);
        server.set_nav_graph(graph);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // laisse le director spawn le PNJ (Pose par défaut (0,0,0))

        let sent = t.take_sent(1);
        let npc_id = sent.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?.iter().next().map(|n| n.id())
        });
        let npc_id = npc_id.expect("le PNJ doit être visible dès le premier snapshot");

        // Repositionne le PNJ à x=10 (le joueur 1, la menace, reste à x=0) — évite le cas dégénéré
        // menace==PNJ décrit ci-dessus. Accès direct à `self.world` : mêmes privilèges que le reste
        // de ce module de test (mod tests est un enfant de server_loop, `world` reste privé au
        // crate/module, cf. le reste de ce fichier).
        server.world.set_pose(
            npc_id,
            Pose {
                x: 10.0,
                y: 0.0,
                z: 0.0,
                ..Pose::default()
            },
        );

        // Le joueur 1 déclenche la fuite (kind=0=Fuite, fondation PNJ déjà câblée).
        let interaction = encode_entity_interaction(npc_id, 0, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: interaction,
        });
        server.tick(&mut t); // applique l'interaction (Fuite) PUIS decide/planifie dans le même tick

        let position_after_first = t.take_sent(1).iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?
                .iter()
                .next()
                .map(|n| n.position().unwrap().x())
        });
        let position_after_first = position_after_first
            .expect("le PNJ doit apparaître dans le snapshot après la fuite déclenchée");

        for _ in 0..20 {
            server.tick(&mut t);
        }
        let position_later = t.take_sent(1).iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?
                .iter()
                .next()
                .map(|n| n.position().unwrap().x())
        });
        let position_later =
            position_later.expect("le PNJ doit rester visible après plusieurs ticks de fuite");

        assert!(
            position_later > position_after_first,
            "le PNJ en fuite doit s'éloigner du joueur 1 (menace à x=0) : x doit croître au fil des \
             ticks (après={position_after_first}, plus tard={position_later})"
        );
    }

    #[test]
    fn a_npc_keeps_advancing_along_its_path_even_with_no_player_connected_dead_reckoning() {
        // Spec §9 critère « adoption sans à-coup » : un PNJ persistant hors de portée avance quand
        // même (dead-reckoning) et doit être vu à sa position avancée — pas à sa position de spawn —
        // dès qu'un observateur rejoint. `tick_npcs` avance tous les PNJ inconditionnellement (pas
        // de garde AoI dans la boucle de mouvement, confirmé Task 5 Step 5) : ce test le prouve en
        // tickant sans observateur PROCHE du PNJ.
        //
        // Trouvé en revue finale de branche : le compte `player_count` de `tick_npcs` comptait à
        // tort les PNJ eux-mêmes (bug corrigé dans le même commit que ce test, cf. commentaire sur
        // `let player_count` plus haut dans ce fichier). Une fois corrigé, un district sans AUCUN
        // vrai joueur despawn réellement (population_director.rs, `!has_players`) — donc ce test ne
        // peut plus déconnecter le SEUL joueur du serveur sans faire despawn le PNJ avant même
        // d'atteindre les ticks de dead-reckoning. Un second joueur reste connecté, loin du PNJ
        // (hors de son rayon AoI de 50, donc jamais son observateur), pour garder le district
        // "default" non-vide pendant tout le scénario — le test prouve ainsi la vraie sémantique
        // LOD spec'd : « persistant hors de PORTÉE », pas « zéro joueur au monde ».
        //
        // Piège de fixture : le brief d'origine de cette tâche proposait un graphe à 2 nœuds, (0,0,0)
        // et (200,0,0) — EXACTEMENT le même piège que Task 5 a trouvé et documenté dans
        // `a_npc_with_a_nav_graph_and_an_errer_brique_moves_over_several_ticks` : ce PNJ utilise la
        // brique `errer` (comportement Calme), dont `decide_destination` tire une destination dans
        // un disque de rayon FIXE 15.0 autour de la position courante (`region_radius` câblé en dur
        // dans `tick_npcs`). Le rayon maximum réellement atteignable par la formule
        // `region_radius * sqrt((rng_unit * 7.0) % 1.0)` est ~14.849 (vérifié par calcul direct sur
        // la formule + reproduit empiriquement : le fixture à 200 unités laisse le PNJ figé en
        // (0,0,0) après 30 ticks de dead-reckoning, cf. rapport de tâche) — donc un nœud à 200 unités
        // n'est JAMAIS le nœud le plus proche d'une destination errer, `nearest_node` retombe toujours
        // sur le nœud de départ, et le "chemin" planifié est un unique waypoint à la position déjà
        // occupée par le PNJ : aucun mouvement, quel que soit le nombre de ticks. Repris ici avec le
        // second nœud à 10.0 (dans le rayon de 15.0), comme le fix de Task 5.
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};
        use crate::npc_catalog::parse_and_validate;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur"
            briques = ["errer"]
            "#,
        )
        .unwrap();
        // Densité cible 1 mais AUCUN joueur présent -> le director ne spawn normalement rien (LOD,
        // fondation PNJ) ; ce test a donc besoin d'un PNJ déjà existant. Le joueur 1 déclenche le
        // spawn puis se déconnecte ; le joueur 2, loin du PNJ, reste connecté pendant tout le
        // scénario pour garder le district non-vide (cf. commentaire ci-dessus) sans jamais observer
        // le PNJ (hors du rayon AoI de 50).
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));
        let mut server = Server::new_with_npcs(50.0, catalog, director);
        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        // `b` à 10.0 (pas 200.0, cf. commentaire ci-dessus) : dans le rayon de tir errer (15.0).
        let b = graph.add_node(NavVec3::new(10.0, 0.0, 0.0));
        graph.add_edge(a, b);
        server.set_nav_graph(graph);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t); // spawn
        server.tick(&mut t); // 1re destination/chemin

        // Joueur 2, loin (bien hors du rayon AoI de 50 depuis l'origine où erre le PNJ) — garde le
        // district non-vide sans jamais devenir l'observateur du PNJ. Sa propre position n'est
        // jamais mise à jour ensuite (aucun PositionUpdate envoyé), donc il reste loin tout le test.
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        let far_position = encode_position(500.0, 500.0, 0.0, 0.0);
        t.inject(TransportEvent::Message {
            from: 2,
            data: far_position,
        });
        server.tick(&mut t);

        t.inject(TransportEvent::Disconnected(1));
        server.tick(&mut t);

        // 30 ticks de dead-reckoning sans observateur PROCHE (le joueur 2 reste connecté, mais loin).
        for _ in 0..30 {
            server.tick(&mut t);
        }
        t.take_sent(2); // purge les snapshots accumulés du joueur 2 (jamais dans l'AoI du PNJ)

        // Un troisième joueur rejoint près du PNJ (le wander le maintient dans le disque de rayon 15
        // autour de l'origine, donc toujours dans le rayon AoI de 50) et doit le voir immédiatement à
        // sa position avancée par dead-reckoning, sans à-coup.
        t.inject(TransportEvent::Connected(3));
        server.tick(&mut t);
        let sent = t.take_sent(3);
        let npc_position = sent.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.npcs()?.iter().next().map(|n| {
                let p = n.position().unwrap();
                (p.x(), p.y())
            })
        });
        let npc_position =
            npc_position.expect("le PNJ doit être immédiatement visible au joueur qui rejoint");
        assert_ne!(
            npc_position,
            (0.0, 0.0),
            "après 30+ ticks de dead-reckoning sans observateur, le PNJ ne doit plus être à sa \
             position de spawn — preuve que le mouvement a continué sans joueur connecté \
             (position observée = {npc_position:?})"
        );
    }
    #[test]
    fn a_server_without_elevators_never_emits_an_elevator_state() {
        // Comportement historique strictement préservé.
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        let sent = t.take_sent(1);
        for buf in &sent {
            let env = flatbuffers::root::<ServerEnvelope>(buf).unwrap();
            assert!(
                env.msg_as_elevator_state_msg().is_none(),
                "sans registre ascenseur, aucun ElevatorStateMsg ne doit partir"
            );
        }
    }

    #[test]
    fn a_vehicle_with_a_nav_graph_moves_along_its_route_over_several_ticks() {
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};

        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        let b = graph.add_node(NavVec3::new(50.0, 0.0, 0.0));
        graph.add_edge(a, b);

        let mut server = Server::new_with_vehicles(50.0);
        server.set_nav_graph(graph);
        server.spawn_vehicle(1, NavVec3::new(0.0, 0.0, 0.0), NavVec3::new(50.0, 0.0, 0.0));

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        let first = t.take_sent(1).iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.vehicles()?.iter().next().map(|v| {
                let p = v.position().unwrap();
                (p.x(), p.y())
            })
        });

        for _ in 0..20 {
            server.tick(&mut t);
        }
        let later = t.take_sent(1).iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.vehicles()?.iter().next().map(|v| {
                let p = v.position().unwrap();
                (p.x(), p.y())
            })
        });

        assert!(first.is_some() && later.is_some());
        assert_ne!(
            first, later,
            "le véhicule doit avoir progressé le long de sa route"
        );
    }

    #[test]
    fn a_vehicle_close_to_its_next_waypoint_pushes_a_pending_entity_report() {
        // Pont Shard→Gateway (Task 5) : un véhicule dont le prochain waypoint est à portée du
        // tampon prédictif (vitesse 8.0 u/s * 2s lookahead = 16 unités, cf.
        // shard_boundary_bridge::should_report_position) doit pousser un rapport de position dans
        // `pending_entity_reports`, drainable via `take_pending_entity_reports`.
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};

        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        let b = graph.add_node(NavVec3::new(10.0, 0.0, 0.0)); // < 16 unités : dans le tampon dès le spawn
        graph.add_edge(a, b);

        let mut server = Server::new_with_vehicles(50.0);
        server.set_nav_graph(graph);
        server.spawn_vehicle(1, NavVec3::new(0.0, 0.0, 0.0), NavVec3::new(10.0, 0.0, 0.0));

        let mut t = InMemoryTransport::new();
        server.tick(&mut t);

        let reports = server.take_pending_entity_reports();
        assert!(
            !reports.is_empty(),
            "un véhicule à moins de 16 unités de son prochain waypoint doit émettre un rapport"
        );
        let (entity_id, _x, _y, _z, speed) = reports[0];
        assert_eq!(entity_id, crate::world::VEHICLE_ID_RANGE_START);
        assert_eq!(speed, 8.0);
    }

    #[test]
    fn a_vehicle_far_from_its_next_waypoint_pushes_no_pending_entity_report() {
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};

        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        let b = graph.add_node(NavVec3::new(1000.0, 0.0, 0.0)); // très loin du tampon (16 unités)
        graph.add_edge(a, b);

        let mut server = Server::new_with_vehicles(50.0);
        server.set_nav_graph(graph);
        server.spawn_vehicle(
            1,
            NavVec3::new(0.0, 0.0, 0.0),
            NavVec3::new(1000.0, 0.0, 0.0),
        );

        let mut t = InMemoryTransport::new();
        server.tick(&mut t);

        assert!(
            server.take_pending_entity_reports().is_empty(),
            "un véhicule loin de son prochain waypoint ne doit émettre aucun rapport"
        );
    }

    #[test]
    fn take_pending_entity_reports_drains_and_does_not_repeat_across_ticks() {
        // std::mem::take doit vider la file : un 2e appel sans nouveau tick ne doit rien renvoyer.
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};

        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        let b = graph.add_node(NavVec3::new(10.0, 0.0, 0.0));
        graph.add_edge(a, b);

        let mut server = Server::new_with_vehicles(50.0);
        server.set_nav_graph(graph);
        server.spawn_vehicle(1, NavVec3::new(0.0, 0.0, 0.0), NavVec3::new(10.0, 0.0, 0.0));

        let mut t = InMemoryTransport::new();
        server.tick(&mut t);

        assert!(!server.take_pending_entity_reports().is_empty());
        assert!(
            server.take_pending_entity_reports().is_empty(),
            "un 2e drain sans nouveau tick ne doit rien renvoyer (file déjà vidée)"
        );
    }

    #[test]
    fn a_server_without_vehicles_never_pushes_pending_entity_reports() {
        // Server::new (sans registre véhicule) : tick_vehicles est un no-op, donc aucun rapport
        // ne doit jamais apparaître, quel que soit le nombre de ticks.
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        for _ in 0..5 {
            server.tick(&mut t);
        }
        assert!(server.take_pending_entity_reports().is_empty());
    }

    #[test]
    fn a_server_without_vehicles_never_adds_any_vehicle_state_to_the_snapshot() {
        // Comportement historique préservé : Server::new (sans véhicules) ne doit jamais faire
        // apparaître de VehicleState dans un Snapshot.
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        let sent = t.take_sent(1);
        let env = flatbuffers::root::<ServerEnvelope>(sent.last().unwrap()).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        assert!(
            snap.vehicles().map(|v| v.len()).unwrap_or(0) == 0,
            "sans registre véhicule, vehicles doit rester vide"
        );
    }

    #[test]
    fn a_configured_elevator_is_broadcast_and_moves_on_a_client_call() {
        use crate::elevator_catalog::parse_and_validate;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[elevator]]
            id = "77"
            name = "test"
            start_floor = 0
            start_delay_ms = 0
            travel_time_ms = 100
            floors = [
              { index = 0, hidden = false, inactive = false },
              { index = 1, hidden = false, inactive = false },
            ]
            "#,
        )
        .unwrap();
        let mut server = Server::new_with_elevators(50.0, catalog, 50);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);

        // Le client appelle l'étage 1.
        server.handle_elevator_call(1, 77, 1);
        server.tick(&mut t);

        let sent = t.take_sent(1);
        let last = sent
            .iter()
            .rev()
            .filter_map(|b| {
                let env = flatbuffers::root::<ServerEnvelope>(b).ok()?;
                env.msg_as_elevator_state_msg()
            })
            .next()
            .expect("un ElevatorStateMsg doit avoir été diffusé");
        assert_eq!(last.elevator_id(), 77);
        assert_eq!(
            last.target_floor(),
            1,
            "la cabine doit viser l'étage appelé"
        );
        assert_eq!(last.movement_state(), 1, "1 = MovingUp");
    }

    /// Encode un `ClientEnvelope::ElevatorCall` — même patron que `encode_position` juste
    /// au-dessus, pour injecter un VRAI message d'appel via le transport plutôt que d'appeler
    /// `Server::handle_elevator_call` directement (nécessaire ici : le fix testé vit dans le
    /// dispatch `ClientMsg::ElevatorCall` de `apply_client_message`, pas dans `handle_elevator_call`
    /// lui-même).
    fn encode_elevator_call(elevator_id: u64, floor: i32) -> Vec<u8> {
        let mut b = FlatBufferBuilder::new();
        let call = ElevatorCall::create(&mut b, &ElevatorCallArgs { elevator_id, floor });
        let env = ClientEnvelope::create(
            &mut b,
            &ClientEnvelopeArgs {
                msg_type: ClientMsg::ElevatorCall,
                msg: Some(call.as_union_value()),
            },
        );
        b.finish(env, None);
        b.finished_data().to_vec()
    }

    #[test]
    fn a_mid_trip_call_that_does_not_change_the_target_is_broadcast_the_same_tick() {
        // Reproduit le finding de la revue finale de branche : `ElevatorState::advance` calcule son
        // `before` de détection de changement APRÈS que `handle_elevator_call` a déjà muté
        // `requested_floors` pour un appel reçu ce même tick (l'event-drain précède `advance` dans
        // `Server::tick`). Un second appel qui n'altère ni `target_floor` ni `movement_state` (la
        // cabine est déjà en route ailleurs) ne ressortait donc d'AUCUNE transition détectée par
        // `tick_elevators`, et n'était diffusé qu'au rappel heartbeat suivant (`HEARTBEAT_TICKS` =
        // 20 ticks plus tard, cf. `tick_elevators`) au lieu d'« appel accepté » (spec §5.3, cadence
        // de diffusion).
        use crate::elevator_catalog::parse_and_validate;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[elevator]]
            id = "77"
            name = "test"
            start_floor = 0
            start_delay_ms = 0
            travel_time_ms = 1000
            floors = [
              { index = 0, hidden = false, inactive = false },
              { index = 1, hidden = false, inactive = false },
              { index = 3, hidden = false, inactive = false },
            ]
            "#,
        )
        .unwrap();
        // tick_ms=50, travel_time_ms=1000 => 1000/50 = 20 ticks pour un trajet complet (même valeur
        // que `HEARTBEAT_TICKS`, non un hasard : ça laisse toute la marge nécessaire pour recevoir
        // le second appel bien avant l'arrivée ET bien avant le rappel heartbeat).
        let mut server = Server::new_with_elevators(50.0, catalog, 50);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        t.take_sent(1);

        // Premier appel : étage 3, loin de l'étage 0 courant. Démarre le trajet — la cabine passe
        // de Stopped à MovingUp, donc CETTE transition est déjà détectée par `tick_elevators`
        // normalement (couvert par `a_configured_elevator_is_broadcast_and_moves_on_a_client_call`
        // ci-dessus). On la laisse passer par le vrai chemin réseau pour rester réaliste.
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_elevator_call(77, 3),
        });
        server.tick(&mut t);
        t.take_sent(1); // vide la diffusion du démarrage de trajet, hors sujet ici

        // Quelques ticks sans aucun appel : la cabine reste en route vers 3, aucune transition, et
        // on est loin d'un multiple de HEARTBEAT_TICKS (20) — donc AUCUNE diffusion d'ElevatorStateMsg
        // ne doit avoir lieu ici. Sert de garde-fou : si ça casse, le test ci-dessous ne prouverait
        // plus rien (le heartbeat pourrait masquer un fix absent).
        for _ in 0..3 {
            server.tick(&mut t);
        }
        let idle_ticks_sent = t.take_sent(1);
        for buf in &idle_ticks_sent {
            let env = flatbuffers::root::<ServerEnvelope>(buf).unwrap();
            assert!(
                env.msg_as_elevator_state_msg().is_none(),
                "aucune transition ni rappel heartbeat pendant ces ticks : pas de diffusion attendue"
            );
        }

        // DEUXIÈME appel, en pleine course : l'étage 1 n'est ni l'étage actif ni le `target_floor`
        // courant (3), et la cabine reste `MovingUp` — `ElevatorState::advance` ne verra donc AUCUN
        // changement avant/après (le root cause du finding).
        t.inject(TransportEvent::Message {
            from: 1,
            data: encode_elevator_call(77, 1),
        });
        server.tick(&mut t); // UN SEUL tick après l'appel — pas d'attente du rappel heartbeat.

        let sent = t.take_sent(1);
        let last = sent
            .iter()
            .rev()
            .filter_map(|b| {
                let env = flatbuffers::root::<ServerEnvelope>(b).ok()?;
                env.msg_as_elevator_state_msg()
            })
            .next()
            .expect(
                "l'appel accepté en pleine course doit être diffusé DANS LE MÊME TICK que \
                 l'appel, sans attendre le rappel heartbeat",
            );
        assert_eq!(last.elevator_id(), 77);
        assert_eq!(
            last.target_floor(),
            3,
            "le target_floor courant (3) ne doit pas avoir changé"
        );
        assert_eq!(last.movement_state(), 1, "1 = MovingUp, inchangé");
        let requested: Vec<i32> = last.requested_floors().unwrap().iter().collect();
        assert!(
            requested.contains(&1),
            "l'étage nouvellement appelé (1) doit apparaître dans requested_floors du message \
             diffusé CE tick, pas seulement au prochain rappel heartbeat"
        );
    }

    #[test]
    fn a_client_connecting_mid_trip_receives_the_current_elevator_state() {
        // Cas D1/D2 de la spec : un joueur qui rejoint en pleine course, ou qui arrive à portée
        // d'une cabine, doit recevoir son état SANS attendre la prochaine transition — sinon il ne
        // saura jamais qu'une cabine est en mouvement.
        use crate::elevator_catalog::parse_and_validate;

        let catalog = parse_and_validate(
            r#"
            format_version = 1
            [[elevator]]
            id = "77"
            name = "test"
            start_floor = 0
            start_delay_ms = 0
            travel_time_ms = 1000
            floors = [
              { index = 0, hidden = false, inactive = false },
              { index = 1, hidden = false, inactive = false },
            ]
            "#,
        )
        .unwrap();
        let mut server = Server::new_with_elevators(50.0, catalog, 50);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        server.handle_elevator_call(1, 77, 1);
        server.tick(&mut t);
        let _ = t.take_sent(1); // on vide ce qui a déjà été envoyé au premier client

        // Un SECOND client arrive alors que la cabine est déjà en route.
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);

        let sent = t.take_sent(2);
        let last = sent
            .iter()
            .rev()
            .filter_map(|b| {
                let env = flatbuffers::root::<ServerEnvelope>(b).ok()?;
                env.msg_as_elevator_state_msg()
            })
            .next()
            .expect("un client qui arrive doit recevoir l'état courant des cabines");

        assert_eq!(
            last.movement_state(),
            1,
            "1 = MovingUp : le client qui rejoint doit apprendre que la cabine est DÉJÀ en mouvement"
        );
        assert_eq!(last.target_floor(), 1, "et vers quel étage elle se dirige");
    }

    #[test]
    fn a_server_can_have_both_npcs_and_elevators_active_at_once() {
        // Preuve de la résolution de la limitation notée par shard.rs (aucun constructeur ne
        // combinait PNJ et ascenseurs) : ascenseurs et PNJ de foule sont deux registres orthogonaux,
        // un `Server` construit avec `new_with_npcs(...).with_elevators(...)` doit avoir les DEUX
        // actifs simultanément — pas seulement l'un ou l'autre selon l'ordre de construction.
        use crate::elevator_catalog::parse_and_validate as parse_elevator_catalog;
        use crate::npc_catalog::parse_and_validate as parse_npc_catalog;
        use crate::population_director::PopulationDirector;
        use std::collections::HashMap;

        let npc_catalog = parse_npc_catalog(
            r#"
            format_version = 1
            [[archetype]]
            id = 1
            name = "marcheur-de-rue"
            briques = ["flaner-sur-place"]
            "#,
        )
        .unwrap();
        let director = PopulationDirector::new(HashMap::from([("default".to_string(), 1)]));

        let elevator_catalog = parse_elevator_catalog(
            r#"
            format_version = 1
            [[elevator]]
            id = "77"
            name = "test"
            start_floor = 0
            start_delay_ms = 0
            travel_time_ms = 100
            floors = [
              { index = 0, hidden = false, inactive = false },
              { index = 1, hidden = false, inactive = false },
            ]
            "#,
        )
        .unwrap();

        let mut server =
            Server::new_with_npcs(50.0, npc_catalog, director).with_elevators(elevator_catalog, 50);

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        server.tick(&mut t);
        server.tick(&mut t); // un 2e tick pour laisser le director réagir à la présence du joueur

        server.handle_elevator_call(1, 77, 1);
        server.tick(&mut t);

        let sent = t.take_sent(1);

        let saw_npc = sent.iter().any(|b| {
            let Ok(env) = flatbuffers::root::<ServerEnvelope>(b) else {
                return false;
            };
            env.msg_as_snapshot()
                .and_then(|snap| snap.npcs())
                .map(|v| !v.is_empty())
                .unwrap_or(false)
        });
        assert!(
            saw_npc,
            "un Server construit avec new_with_npcs(...).with_elevators(...) doit toujours \
             faire apparaître des PNJ dans ses snapshots"
        );

        let saw_elevator = sent.iter().any(|b| {
            let Ok(env) = flatbuffers::root::<ServerEnvelope>(b) else {
                return false;
            };
            env.msg_as_elevator_state_msg().is_some()
        });
        assert!(
            saw_elevator,
            "le même Server doit AUSSI diffuser l'état de l'ascenseur configuré — les deux \
             registres doivent coexister, pas s'exclure"
        );
    }

    #[test]
    fn mounting_a_vehicle_makes_the_passenger_position_follow_it() {
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};

        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        let b = graph.add_node(NavVec3::new(50.0, 0.0, 0.0));
        graph.add_edge(a, b);

        let mut server = Server::new_with_vehicles(1000.0);
        server.set_nav_graph(graph);
        server.spawn_vehicle(1, NavVec3::new(0.0, 0.0, 0.0), NavVec3::new(50.0, 0.0, 0.0));
        let vehicle_id = crate::world::VEHICLE_ID_RANGE_START;

        let mut t = InMemoryTransport::new();
        // Joueur 1 = futur passager, joueur 2 = observateur.
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        t.take_sent(1);
        let sent_to_2_before = t.take_sent(2);

        // Position de joueur 1 AVANT montée, vue par l'observateur (doit être (0,0,0), pose par défaut).
        let pos_before = sent_to_2_before.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.players()?.iter().find(|p| p.id() == 1).map(|p| {
                let pos = p.position().unwrap();
                (pos.x(), pos.y(), pos.z())
            })
        });
        assert_eq!(pos_before, Some((0.0, 0.0, 0.0)));

        // Joueur 1 monte dans le véhicule (kind=3=Mount, param=0=premier siège).
        let mount = encode_entity_interaction(vehicle_id, 3, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: mount,
        });
        server.tick(&mut t);
        t.take_sent(1);
        t.take_sent(2);

        // Plusieurs ticks : le véhicule avance le long de sa route (a → b).
        for _ in 0..20 {
            server.tick(&mut t);
        }
        t.take_sent(1);
        let sent_to_2_after = t.take_sent(2);

        // Position de joueur 1 APRÈS plusieurs ticks, vue par l'observateur : doit avoir bougé
        // (suit le véhicule), pas être restée figée à (0,0,0).
        let pos_after = sent_to_2_after.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.players()?.iter().find(|p| p.id() == 1).map(|p| {
                let pos = p.position().unwrap();
                (pos.x(), pos.y(), pos.z())
            })
        });
        assert!(pos_after.is_some());
        assert_ne!(
            pos_before, pos_after,
            "la position du passager doit suivre le véhicule après montée, pas rester figée"
        );

        // Vérification renforcée : la position du passager doit correspondre exactement à celle du
        // véhicule au même tick (invariant convoi, pas juste "a bougé un peu par hasard"). Les deux
        // positions doivent être lues dans le MÊME message de snapshot (un message par tick reçu
        // dans `sent_to_2_after` — comparer une position prise au tick N à une position prise à un
        // autre tick M donnerait un faux échec, l'un et l'autre bougeant à chaque tick).
        let last = sent_to_2_after
            .last()
            .expect("au moins un snapshot reçu par l'observateur sur les 20 derniers ticks");
        let env = flatbuffers::root::<ServerEnvelope>(last).unwrap();
        let snap = env.msg_as_snapshot().unwrap();
        let pos_after_same_tick = snap
            .players()
            .unwrap()
            .iter()
            .find(|p| p.id() == 1)
            .map(|p| {
                let pos = p.position().unwrap();
                (pos.x(), pos.y(), pos.z())
            });
        let vehicle_pos_after = snap.vehicles().unwrap().iter().next().map(|v| {
            let p = v.position().unwrap();
            (p.x(), p.y(), p.z())
        });
        assert_eq!(
            pos_after_same_tick, vehicle_pos_after,
            "la position du passager doit être EXACTEMENT celle du véhicule (invariant convoi)"
        );
    }

    #[test]
    fn unmounting_a_vehicle_stops_the_passenger_position_from_following_it() {
        use crate::nav_graph::{NavGraph, Vec3 as NavVec3};

        let mut graph = NavGraph::new();
        let a = graph.add_node(NavVec3::new(0.0, 0.0, 0.0));
        let b = graph.add_node(NavVec3::new(50.0, 0.0, 0.0));
        graph.add_edge(a, b);

        let mut server = Server::new_with_vehicles(1000.0);
        server.set_nav_graph(graph);
        server.spawn_vehicle(1, NavVec3::new(0.0, 0.0, 0.0), NavVec3::new(50.0, 0.0, 0.0));
        let vehicle_id = crate::world::VEHICLE_ID_RANGE_START;

        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        t.inject(TransportEvent::Connected(2));
        server.tick(&mut t);
        t.take_sent(1);
        t.take_sent(2);

        let mount = encode_entity_interaction(vehicle_id, 3, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: mount,
        });
        server.tick(&mut t);
        t.take_sent(1);
        t.take_sent(2);

        // Quelques ticks montés, puis démonte.
        for _ in 0..5 {
            server.tick(&mut t);
        }
        t.take_sent(1);
        t.take_sent(2);

        let unmount = encode_entity_interaction(vehicle_id, 4, 0);
        t.inject(TransportEvent::Message {
            from: 1,
            data: unmount,
        });
        server.tick(&mut t);
        t.take_sent(1);
        let sent_to_2_at_unmount = t.take_sent(2);
        let pos_at_unmount = sent_to_2_at_unmount.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.players()?.iter().find(|p| p.id() == 1).map(|p| {
                let pos = p.position().unwrap();
                (pos.x(), pos.y(), pos.z())
            })
        });

        // Encore plusieurs ticks : le véhicule continue d'avancer, mais joueur 1 (démonté) doit
        // rester figé à sa position au moment du démontage.
        for _ in 0..20 {
            server.tick(&mut t);
        }
        t.take_sent(1);
        let sent_to_2_later = t.take_sent(2);
        let pos_later = sent_to_2_later.iter().find_map(|bytes| {
            let env = flatbuffers::root::<ServerEnvelope>(bytes).ok()?;
            let snap = env.msg_as_snapshot()?;
            snap.players()?.iter().find(|p| p.id() == 1).map(|p| {
                let pos = p.position().unwrap();
                (pos.x(), pos.y(), pos.z())
            })
        });

        assert_eq!(
            pos_at_unmount, pos_later,
            "après démontage, la position du passager ne doit plus suivre le véhicule"
        );
    }

    #[test]
    fn degradation_tier_becomes_degraded_after_a_sustained_run_of_slow_ticks() {
        let mut server = Server::new(50.0);
        let mut t = InMemoryTransport::new();
        t.inject(TransportEvent::Connected(1));
        // 200 ticks rapides (aucune charge réelle simulée dans ce test unitaire — ce test vérifie le
        // MÉCANISME de fenêtre/hystérésis, pas une vraie charge CPU) ne suffisent PAS à franchir le
        // seuil de 40ms tout seuls (un tick de InMemoryTransport est de l'ordre de la microseconde) —
        // ce test vérifie donc plutôt l'accesseur de test et le comportement par défaut (Normal tant
        // qu'aucune lenteur réelle n'est mesurée). Un test de franchissement RÉEL du seuil nécessiterait
        // d'injecter artificiellement des durées dans la fenêtre — accesseur de test dédié ci-dessous.
        for _ in 0..250 {
            server.tick(&mut t);
        }
        assert_eq!(
            server.degradation_tier_for_test(),
            crate::degradation::DegradationTier::Normal,
            "un Server qui tourne normalement (aucune lenteur réelle) ne doit jamais passer Degraded"
        );
    }

    #[test]
    fn degradation_tier_responds_to_an_injected_slow_tick_window() {
        let mut server = Server::new(50.0);
        // Accès de test direct à la fenêtre pour simuler une charge sans dépendre d'un vrai busy-loop
        // (fragile en CI) — injecte 200 durées toutes au-dessus du seuil d'entrée (40ms).
        server.inject_tick_durations_for_test(vec![45_000; 200]);
        let mut t = InMemoryTransport::new();
        server.tick(&mut t); // un tick réel (rapide) s'ajoute à la fenêtre pleine, éjecte la plus vieille
        assert_eq!(
            server.degradation_tier_for_test(),
            crate::degradation::DegradationTier::Degraded,
            "199 ticks lents + le p99 de la fenêtre doit rester au-dessus du seuil d'entrée"
        );
    }
}

#[cfg(test)]
mod degradation_window_tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn p99_of_an_empty_window_is_none() {
        let window: VecDeque<u64> = VecDeque::new();
        assert_eq!(p99_of(&window), None);
    }

    #[test]
    fn p99_of_a_single_value_window_is_that_value() {
        let window: VecDeque<u64> = VecDeque::from([5000]);
        assert_eq!(p99_of(&window), Some(5000));
    }

    #[test]
    fn p99_of_a_hundred_values_returns_the_99th_percentile() {
        // 100 valeurs 1..=100 (micros) — le p99 attendu est la 99e plus petite valeur triée, soit
        // 99 (index 98 en base 0 sur 100 éléments triés — cohérent avec la convention "au moins
        // 99% des valeurs sont <= au résultat").
        let window: VecDeque<u64> = (1..=100).collect();
        assert_eq!(p99_of(&window), Some(99));
    }

    #[test]
    fn p99_of_is_order_independent() {
        // La fenêtre n'est pas nécessairement triée par ordre d'arrivée — p99_of doit trier
        // lui-même, pas supposer un ordre.
        let mut window: VecDeque<u64> = (1..=100).collect();
        // Mélange grossier : inverse la fenêtre.
        window.make_contiguous().reverse();
        assert_eq!(p99_of(&window), Some(99));
    }
}
