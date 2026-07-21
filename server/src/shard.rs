//! Le Shard : simulation autoritaire d'une zone, pilotée par le Gateway via TCP interne.
//! Une seule connexion Gateway en v1 (M0-M1). Tick 20 Hz.

use crate::elevator_catalog::ElevatorCatalog;
use crate::internal_net::InternalTransport;
use crate::named_npc_catalog::NamedNpcCatalog;
use crate::named_npc_registry::NamedNpcRegistry;
use crate::nav_graph::{NavGraph, Vec3 as NavVec3};
use crate::npc_catalog::NpcCatalog;
use crate::population_director::PopulationDirector;
use crate::server_loop::Server;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Cadence réelle du Shard, en millisecondes — 20 Hz. Constante nommée (plutôt qu'un littéral
/// dupliqué) car `Server::with_elevators`/`new_with_elevators` (Task 6) ont besoin de cette même
/// valeur pour convertir leurs durées de trajet (`travel_time_ms`, `start_delay_ms`) en nombre de
/// ticks — `TICK` (la version `Duration` utilisée par le ticker tokio) en dérive pour ne jamais
/// diverger.
pub(crate) const TICK_MS: u32 = 50;
const TICK: Duration = Duration::from_millis(TICK_MS as u64); // 20 Hz

/// Graine déclarative pour faire naître des véhicules autonomes dans un vrai process Shard, sans
/// exiger que `NavGraph` (qui ne dérive PAS `Clone`) soit copiable. `shard_main` reconstruit un
/// `NavGraph` neuf à partir de cette graine à CHAQUE (re)connexion Gateway — cohérent avec le fait
/// que le `Server` est lui-même reconstruit par connexion (l'état de simulation d'un Shard n'est
/// pas persistant ; il repart de zéro à chaque nouvelle connexion Gateway, cf. la boucle `accept`
/// ci-dessous et le commentaire "réinitialisation du shard"). Purement additif : les 3 voies
/// existantes (`new_with_npcs`/`new_with_named_npcs`/`new_with_metrics`) restent inchangées et
/// prioritaires ; la voie véhicule n'est empruntée que quand ni `population` ni `named_npc` ne sont
/// fournis ET qu'une graine véhicule l'est.
///
/// Aujourd'hui n'a pour appelant que le test d'intégration réseau réel cross-shard
/// (`tests/vehicle_predictive_handoff.rs`) : le câblage production (director de trafic qui
/// peuplerait un Shard depuis un manifeste) est différé (spec véhicules autonomes §2, trafic
/// d'ambiance). `bin/shard.rs` passe donc `None` — comportement de production strictement préservé.
#[derive(Clone, Debug)]
pub struct VehicleShardSeed {
    /// Positions des nœuds du graphe de navigation, indexées comme les `NodeId` rendus par
    /// `NavGraph::add_node` (ordre d'insertion : le nœud `i` = `nodes[i]`).
    pub nodes: Vec<NavVec3>,
    /// Arêtes non orientées (paires d'indices dans `nodes`) — pondérées par distance euclidienne
    /// au moment de la reconstruction, exactement comme `NavGraph::add_edge`.
    pub edges: Vec<(usize, usize)>,
    /// Véhicules à faire naître : `(archétype, from, to)` — même triplet que `Server::spawn_vehicle`.
    pub vehicles: Vec<(u32, NavVec3, NavVec3)>,
}

impl VehicleShardSeed {
    /// Reconstruit un `NavGraph` frais à partir de la graine (nécessaire à chaque connexion car
    /// `NavGraph` n'est pas `Clone` et le `Server` est recréé par connexion).
    fn build_graph(&self) -> NavGraph {
        let mut graph = NavGraph::new();
        for node in &self.nodes {
            graph.add_node(*node);
        }
        for (a, b) in &self.edges {
            graph.add_edge(*a, *b);
        }
        graph
    }
}

pub async fn shard_main(
    addr: &str,
    aoi_radius: f32,
    metrics_addr: &str,
    population: Option<(NpcCatalog, PopulationDirector)>,
    named_npc: Option<(NamedNpcCatalog, NamedNpcRegistry)>,
    elevators: Option<ElevatorCatalog>,
    vehicles: Option<VehicleShardSeed>,
) -> std::io::Result<()> {
    let metrics = crate::metrics::Metrics::new();
    {
        let metrics = metrics.clone();
        let metrics_addr = metrics_addr.to_string();
        tokio::spawn(async move {
            if let Err(e) = crate::metrics::serve(&metrics_addr, metrics).await {
                tracing::warn!("endpoint métriques indisponible ({metrics_addr}): {e}");
            }
        });
    }

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Shard en écoute (interne) sur {addr}");
    // Enregistré UNE SEULE FOIS avant la boucle, comme côté Gateway (cf. shutdown.rs) — une
    // reconstruction par itération perdrait un signal reçu entre deux itérations.
    let mut shutdown = crate::shutdown::ShutdownSignal::new();
    loop {
        let (mut sock, peer) = tokio::select! {
            accepted = listener.accept() => accepted?,
            _ = shutdown.recv() => {
                tracing::info!("Arrêt propre : extinction du Shard");
                return Ok(());
            }
        };
        tracing::info!("Gateway connecté depuis {peer}");
        // `new_with_metrics` (Task 6, observabilité) : `Server::tick` enregistre lui-même la durée
        // de chaque tick dans l'histogramme `tessera_tick_duration`/le compteur `overruns_total` —
        // en plus (pas à la place) de `last_tick_micros` ci-dessous, qui reste la gauge "dernier
        // tick" existante consommée par le dashboard/l'alerte ShardFrozen.
        //
        // `new_with_npcs`/`new_with_named_npcs` n'acceptent pas encore `metrics` (limitation de
        // Task 6, déjà revue — pas rouverte ici) : un Shard avec `[runtime.population]` et/ou
        // `named_npc_manifest_path` configuré perd donc l'histogramme
        // `tessera_tick_duration`/`overruns_total` interne à `Server::tick` tant que cette
        // fondation n'a pas de constructeur combinant les trois. Les gauges
        // `last_tick_micros`/`players` ci-dessous restent actives dans tous les cas (mesurées ici,
        // hors de `Server`).
        //
        // PNJ de foule (`population`) et PNJ nominatifs (`named_npc`) sont deux registres distincts
        // sur `Server` (`npc_registry`/`named_npc_registry`, cf. server_loop.rs) mais aucun
        // constructeur ne les active tous les deux à la fois pour l'instant (limitation de cette
        // fondation, pas un oubli — un Shard avec les deux configurés active seulement les PNJ de
        // foule ; raffinement futur si le besoin apparaît). `named_npc` est vérifié en second pour
        // que `population` garde la priorité, comme avant l'existence de cette fonctionnalité.
        let mut server = match (&population, &named_npc) {
            (Some((catalog, director)), _) => {
                Server::new_with_npcs(aoi_radius, catalog.clone(), director.clone())
            }
            (None, Some((catalog, registry))) => {
                Server::new_with_named_npcs(aoi_radius, catalog, registry.clone())
            }
            // Voie véhicule (spec véhicules autonomes §5) : n'est empruntée que si NI foule NI PNJ
            // nominatif ne sont configurés — les 3 voies existantes gardent la priorité et restent
            // strictement inchangées. Le graphe de nav et les véhicules sont reconstruits à neuf à
            // chaque connexion depuis la graine `Clone` (cf. doc de `VehicleShardSeed` — `NavGraph`
            // n'est pas `Clone`, et l'état de simulation du Shard n'est de toute façon pas
            // persistant entre connexions).
            (None, None) => match &vehicles {
                Some(seed) => {
                    let mut s = Server::new_with_vehicles(aoi_radius);
                    s.set_nav_graph(seed.build_graph());
                    for (archetype, from, to) in &seed.vehicles {
                        s.spawn_vehicle(*archetype, *from, *to);
                    }
                    s
                }
                None => Server::new_with_metrics(aoi_radius, metrics.clone()),
            },
        };
        // Ascenseurs (Task 6) : orthogonaux aux PNJ (foule ou nominatifs), donc chaînés APRÈS le
        // match ci-dessus plutôt qu'intégrés dedans — `Server::with_elevators` s'applique quel
        // qu'ait été le constructeur choisi pour les PNJ, contrairement à la limitation
        // population/named_npc documentée juste au-dessus (celle-là reste entière, elle ne
        // concerne pas les ascenseurs).
        if let Some(catalog) = &elevators {
            server = server.with_elevators(catalog.clone(), TICK_MS);
        }
        let mut transport = InternalTransport::new();
        let mut buf = [0u8; 8192];
        let mut ticker = tokio::time::interval(TICK);
        // Défaut tokio = Burst : un tick en retard déclenche une rafale de rattrapage — chaque
        // tick de rattrapage ré-encode et renvoie un snapshot complet à chaque joueur, dépensant
        // donc PLUS de CPU/réseau juste après un pic de charge. Skip saute les ticks manqués
        // (le dernier état écrase les précédents, cf. audit prod 2026-07-03 §4.3/§C.2).
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                // Lecture des frames du Gateway (events clients).
                read = sock.read(&mut buf) => {
                    let n = match read { Ok(0) | Err(_) => break, Ok(n) => n };
                    if !transport.feed(&buf[..n]) {
                        tracing::warn!("frame surdimensionné reçu du Gateway — connexion fermée");
                        break;
                    }
                }
                // Tick de simulation 20 Hz.
                _ = ticker.tick() => {
                    let tick_start = std::time::Instant::now();
                    server.tick(&mut transport);
                    metrics.last_tick_micros.store(
                        tick_start.elapsed().as_micros() as i64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    metrics.players.store(
                        server.player_count() as u64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    for frame in transport.take_outbound() {
                        if sock.write_all(&frame).await.is_err() {
                            return Ok(()); // Gateway parti
                        }
                    }
                    // Pont Shard→Gateway générique (shard_boundary_bridge.rs, spec véhicules
                    // autonomes §5) : un véhicule dont le chemin planifié entre dans le tampon
                    // prédictif d'un shard voisin a poussé un rapport dans `tick_vehicles` ce
                    // tick — on le draine et l'écrit sur la même socket TCP interne que les
                    // frames `ServerSend` ci-dessus (multiplexé, `InternalEnvelope` distingue les
                    // deux types côté lecteur). Même garde de déconnexion Gateway que ci-dessus.
                    for (entity_id, x, y, z, speed) in server.take_pending_entity_reports() {
                        let frame =
                            crate::internal_net::encode_entity_position_report(entity_id, x, y, z, speed);
                        if sock.write_all(&frame).await.is_err() {
                            return Ok(()); // Gateway parti
                        }
                    }
                }
                // Arrêt propre : ne pas attendre la fin de la connexion Gateway pour répondre
                // au signal (sinon un SIGTERM/SIGINT reste bloqué tant qu'un Gateway est connecté).
                _ = shutdown.recv() => {
                    tracing::info!("Arrêt propre : extinction du Shard");
                    return Ok(());
                }
            }
        }
        tracing::info!("Gateway déconnecté, réinitialisation du shard");
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    // Envoie un signal Unix réel au process de test courant — sûr ici car `ShutdownSignal`
    // installe un handler qui remplace la disposition par défaut (le process ne meurt pas),
    // le futur `recv()` se contente de se résoudre. Passe par la commande `kill` plutôt qu'une
    // dépendance `libc` pour ne pas toucher Cargo.toml (hors périmètre de cette tâche).
    #[cfg(unix)]
    fn send_self_sigterm() {
        let pid = std::process::id().to_string();
        let status = std::process::Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .expect("kill -TERM devrait pouvoir s'exécuter");
        assert!(status.success(), "kill -TERM a échoué");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shard_flushes_nothing_but_exits_cleanly_on_shutdown_signal() {
        // Le Shard n'a pas de persistance propre (state Gateway-autoritaire) — le test vérifie
        // seulement que la boucle sort proprement (retourne Ok(())) sur réception du signal,
        // sans laisser de connexion Gateway en attente indéfiniment (ici : aucune connexion du
        // tout — la course doit gagner dès la boucle d'accept externe).
        let addr = "127.0.0.1:27131";
        let handle = tokio::spawn(async move {
            super::shard_main(addr, 1000.0, "127.0.0.1:0", None, None, None, None).await
        });

        // Laisse le shard se binder et enregistrer son ShutdownSignal avant d'envoyer le signal.
        tokio::time::sleep(Duration::from_millis(200)).await;

        send_self_sigterm();

        let result = tokio::time::timeout(Duration::from_secs(3), handle)
            .await
            .expect("shard_main aurait dû sortir avant le timeout au lieu d'attendre une connexion")
            .expect("la tâche du shard n'aurait pas dû paniquer");

        assert!(
            result.is_ok(),
            "shard_main devrait retourner Ok(()) sur arrêt propre"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shard_flushes_cleanly_on_shutdown_signal_while_a_connection_is_active() {
        // Contrairement au test précédent (aucune connexion), ici une vraie connexion
        // Gateway est établie avant l'envoi du signal — le shard est donc entré dans la
        // boucle interne par-connexion. Ce test prouve que la course d'arrêt y est aussi
        // gagnée (l'arm `shutdown.recv()` du `select!` interne), et que le shard ne reste
        // pas bloqué à attendre la fin de la connexion Gateway. Port différent de
        // 127.0.0.1:27131 (utilisé par le test ci-dessus) pour éviter toute collision entre
        // tests exécutés en parallèle.
        let addr = "127.0.0.1:27132";
        let handle = tokio::spawn(async move {
            super::shard_main(addr, 1000.0, "127.0.0.1:0", None, None, None, None).await
        });

        // Laisse le shard se binder et enregistrer son ShutdownSignal avant de se connecter.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let _sock = tokio::net::TcpStream::connect(addr)
            .await
            .expect("la connexion TCP au shard devrait réussir");

        // Laisse le shard accepter la connexion et entrer dans la boucle interne
        // par-connexion avant d'envoyer le signal.
        tokio::time::sleep(Duration::from_millis(200)).await;

        send_self_sigterm();

        let result = tokio::time::timeout(Duration::from_secs(3), handle)
            .await
            .expect(
                "shard_main aurait dû sortir avant le timeout au lieu d'attendre la fin \
                 de la connexion Gateway",
            )
            .expect("la tâche du shard n'aurait pas dû paniquer");

        assert!(
            result.is_ok(),
            "shard_main devrait retourner Ok(()) sur arrêt propre même avec une connexion active"
        );
    }
}
