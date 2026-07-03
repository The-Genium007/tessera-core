# ADR 0003 : Binding Rust pour GameNetworkingSockets (GNS)

- **Statut :** accepté
- **Date :** 2026-06-26

## Contexte

Le serveur Rust utilise un trait `Transport` poll-based pour abstraire le réseau.
`GnsTransport` doit implémenter ce trait en s'appuyant sur la bibliothèque C++ Valve
**GameNetworkingSockets** (GNS). Un spike a prouvé que le binding Rust compile sur
macOS arm64 et établit une connexion loopback, afin de figer l'API avant d'écrire
l'implémentation du transport.

### Candidats évalués

| Crate | Version testée | Résultat |
|---|---|---|
| `game-networking-sockets` (lib name `gns`) | 0.2.0 | **Build + loopback OK** |
| `gns` (hussein-aitlahcen, crates.io) | 0.1.0 | Non testé — même auteur, même code, ancienne version |
| `gamenetworkingsockets-rs` (restitux) | 0.1.0 | Non testé — 3 commits, abandonné |

La crate `game-networking-sockets = "0.2.0"` (publiée le 2026-06-02) est la version
courante et activement maintenue du dépôt `github.com/hussein-aitlahcen/gns-rs`. Son
`[lib] name = "gns"` signifie que dans le code Rust on l'importe via `use gns::...`.

## Décision

**Crate retenue : `game-networking-sockets = "0.2.0"`** (lib name `gns`), dépôt
`hussein-aitlahcen/gns-rs`.

Ajout dans `Cargo.toml` :

```toml
game-networking-sockets = "0.2"
```

Import dans le code Rust :

```rust
use gns::{GnsGlobal, GnsSocket, GnsConnection, MessageSlot, SendFlags};
use gns::sys::ESteamNetworkingConnectionState;
```

## Prérequis de build macOS (arm64)

Les commandes suivantes doivent être exécutées une seule fois :

```bash
brew install cmake pkgconf flatbuffers
brew install protobuf@21          # version 3.21.x requise ; protobuf >= 4 casse le build GNS
```

Variables d'environnement à positionner pour `cargo build` (à placer dans `.env`, un
wrapper script, ou `~/.cargo/config.toml`) :

```bash
export PATH="/opt/homebrew/opt/protobuf@21/bin:$PATH"
export PKG_CONFIG_PATH="/opt/homebrew/opt/protobuf@21/lib/pkgconfig:/opt/homebrew/opt/openssl@3/lib/pkgconfig"
export OPENSSL_ROOT_DIR="/opt/homebrew/opt/openssl@3"
```

`openssl@3` est déjà installé par Homebrew en standard sur les machines Apple Silicon
récentes.

## API figée — appels exacts

Toutes les signatures ci-dessous proviennent du spike loopback (compilé et exécuté sur
macOS arm64).

### (a) Créer un endpoint d'écoute

```rust
// Initialiser le singleton global GNS (une seule fois par process).
let gns_global: &'static GnsGlobal = GnsGlobal::get().expect("GnsGlobal::get failed");

// Ouvrir un socket en écoute sur 127.0.0.1:<port> (ou 0.0.0.0 pour toutes interfaces).
use std::net::Ipv4Addr;
let server: GnsSocket<IsServer> = GnsSocket::new(gns_global)
    .listen(Ipv4Addr::LOCALHOST.into(), port)
    .expect("listen failed");
```

`IsServer` est un type-état : le socket expose uniquement les opérations serveur après
`listen`.

### (b) Connecter un client

```rust
let client: GnsSocket<IsClient> = GnsSocket::new(gns_global)
    .connect(Ipv4Addr::LOCALHOST.into(), port)
    .expect("connect failed");

// La connexion est asynchrone. Poller les events jusqu'à Connected (voir § c).
let conn_handle: GnsConnection = client.connection(); // handle stable pour send
```

`IsClient` est un autre type-état : le socket expose uniquement les opérations client
après `connect`.

### (c) Poller les événements (Connected / Disconnected / Message)

La boucle de tick doit appeler **les trois opérations suivantes** à chaque itération :

```rust
// 1. Callbacks bas niveau GNS — OBLIGATOIRE à chaque tick.
gns_global.poll_callbacks();

// 2. Événements de connexion (Connected, Disconnected).
for event in socket.receive_events() {
    match (event.old_state(), event.info().state()) {
        // Nouvelle connexion entrante (côté serveur uniquement).
        (
            ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_None,
            ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
        ) => {
            server.accept(event.connection()).expect("accept failed");
            // event.connection() : GnsConnection — ID stable pour la durée de vie.
        }

        // Client connecté (côté client — confirmation de connexion).
        (
            ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connecting,
            ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_Connected,
        ) => { /* connexion établie */ }

        // Fermeture.
        (_, ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ClosedByPeer
            | ESteamNetworkingConnectionState::k_ESteamNetworkingConnectionState_ProblemDetectedLocally) => {
            let _ = server.close_connection(event.connection(), 0, None, false);
        }

        _ => {}
    }
}

// 3. Messages entrants (poll groupe côté serveur, connexion côté client).
// Variante zero-allocation (buffer réutilisé) :
let mut recv_buf = [const { MessageSlot::uninit() }; 128];
for msg in socket.receive_messages_into(&mut recv_buf).expect("receive failed") {
    let payload: &[u8] = msg.payload();
    let conn: GnsConnection = msg.connection(); // connexion source (côté serveur)
}

// Variante inline (buffer K inline dans l'itérateur) :
for msg in socket.receive_messages::<128>().expect("receive failed") {
    let payload: &[u8] = msg.payload();
}
```

**Mapping `GnsConnection` → `ClientId` :** `GnsConnection` enveloppe un `u32` opaque
(`HSteamNetConnection`) mais son champ interne est **privé** dans
`game-networking-sockets = 0.2.0` (donc `conn.0` n'est pas accessible). `GnsConnection`
dérive en revanche `Copy + Hash + Eq` : on l'utilise **directement comme clé de HashMap**.
Le transport assigne son **propre `ClientId` monotone** (compteur démarré à 1) à chaque
connexion acceptée, et maintient deux maps (`id_to_conn` / `conn_to_id`) pour la
correspondance dans les deux sens — aucun `transmute`, aucun accès à un champ privé.
L'handle `GnsConnection` reste stable pour toute la durée de vie de la connexion.

### (d) Envoyer un message fiable à une connexion

```rust
// Allouer + envoyer un message fiable (Vec<u8>, String, &'static [u8], etc.).
let msg = gns_global.utils().allocate_message(
    connection,          // GnsConnection
    SendFlags::RELIABLE, // ou SendFlags::UNRELIABLE
    data,                // impl Payload : Vec<u8>, Box<[u8]>, String, Arc<[u8]>, &'static [u8]
);
socket.send_messages(vec![msg]);

// Variante one-message avec résultat :
let _msg_number: u64 = socket.send_message(msg).expect("send failed");
```

`allocate_message` prend ownership de `data` ; GNS libère la mémoire quand le message a
été envoyé via le trait `Payload`.

## Spike de référence

Spike loopback local (hors dépôt). Sortie observée :

```
[server] accepted connection GnsConnection(3646190072)
[client] connected
[client] sent reliable message
[server] received: hello from spike loopback

[SPIKE OK] Server received: "hello from spike loopback"
[SPIKE] loopback test passed -- GNS binding works on macOS (arm64)
```

## Conséquences

**Positives :**
- Le binding compile sur macOS arm64 (Apple Silicon) avec les prérequis brew listés.
- L'API est pure Rust, type-safe, sans unsafe visible côté utilisateur.
- Le type-état (`IsServer` / `IsClient`) empêche les erreurs d'appel au niveau de la
  compilation.
- `GnsConnection` est un identifiant stable pouvant servir de `ClientId` directement.

**Négatives / Risques :**
- `protobuf@21` est déprécié dans Homebrew (désactivé en janvier 2026 pour les nouvelles
  formules, mais le bottle existe encore). Si la bottle disparaît, il faudra builder
  protobuf@21 depuis les sources ou basculer vers un cmake manuel sans pkg-config.
- Le build nécessite `cmake` + `clang` + les outils Xcode Command Line Tools — une CI
  macOS doit avoir ces outils installés.
- Non testé sur macOS Intel (x86_64). La crate indique « Intel non testé ».
- `GnsGlobal` est un singleton `OnceLock` : pas de cycle init/kill dans le même process ;
  cela ne pose pas de problème pour un serveur long-running.

## Alternatives considérées

- **`gns = "0.1.0"` (crates.io)** : ancienne version du même code source, remplacée par
  `game-networking-sockets = "0.2.0"` — écarté.
- **`gamenetworkingsockets-rs` (restitux)** : 3 commits, pas de release, abandonné —
  écarté.
- **Binding `-sys` maison (cmake + cxx)** : coût élevé, non nécessaire vu que
  `game-networking-sockets` compile proprement.
- **Différer le test réseau sur Windows** : possible si le spike avait échoué, mais le
  loopback macOS fonctionne — écarté.
