# Contrat d'intégration client-mod ↔ serveur Rust (tranche verticale 0-D)

Ce que le client-mod porté (RED4ext + redscript) doit implémenter pour parler à notre serveur
autoritaire. **Tout existe et est testé côté serveur** (milestone « 2 clients se voient bouger »,
`server/src/server_loop.rs`) ; ce document fige le contrat que le port Windows doit cibler.

> Décisions liées : transport **GNS** (D5), protocole **FlatBuffers** (D7), serveur **Rust** (D1),
> port du client Cyberverse ([ADR 0005](../docs/architecture/0005-cyberverse-port-vs-rebuild.md)).

## Connexion

- **Transport** : GameNetworkingSockets, **UDP fiable** (`k_nSteamNetworkingSend_Reliable`).
- **Serveur** : écoute sur une adresse `ip:port`, **défaut `127.0.0.1:27020`**
  (`server/src/main.rs`, lancé avec `--features gns`).
- **Adresse côté client** : passée au jeu en **ligne de commande** (le client Cyberverse parse déjà
  `GetCommandLineA()` dans `OnNetworkUpdate`) → on réutilise ce mécanisme, le launcher injectera l'IP.
- Le serveur attribue un `ClientId` (u64) au niveau transport à la connexion (`TransportEvent::Connected`).

## Format de fil (FlatBuffers)

Schéma : `protocol/schema/protocol.fbs`, namespace `cyberpunk_rp.protocol`.

```fbs
struct Vec3 { x:float; y:float; z:float; }
table Join { display_name:string; }
table PositionUpdate { position:Vec3; yaw:float; }
table PlayerState { id:ulong; position:Vec3; yaw:float; }
table Snapshot { tick:ulong; players:[PlayerState]; }
union ClientMsg { Join, PositionUpdate }   // root: ClientEnvelope{ msg }
union ServerMsg { Snapshot }               // root: ServerEnvelope{ msg }
```

Chaque message sur le fil = un `ClientEnvelope` (client→serveur) ou `ServerEnvelope` (serveur→client)
fini comme buffer FlatBuffers racine. Côté C++, générer les en-têtes avec
`flatc --cpp -o <out> protocol/schema/protocol.fbs` (flatc **25.12.19**, aligné sur le crate Rust).

## Flux (tranche verticale)

**Client → serveur**
1. À la connexion : envoyer `ClientEnvelope(Join{ display_name })` une fois.
2. À chaque frame/tick réseau : envoyer `ClientEnvelope(PositionUpdate{ position, yaw })` avec la
   position du joueur local (`Utils.h::GetWorldPosition` côté Cyberverse).

**Serveur → client** (tick **20 Hz**, `default_tick_rate_hz()`)
3. À chaque tick, le serveur envoie à CE client un `ServerEnvelope(Snapshot{ tick, players })`.
   ⚠️ **Le snapshot exclut déjà le joueur lui-même** (`world.rs::snapshot_for`) : `players` ne
   contient que les **autres** joueurs. Le client n'a donc PAS besoin de connaître son propre id
   pour cette tranche.
4. À la réception d'un `Snapshot` : pour chaque `PlayerState` →
   - id inconnu → **spawn** une entité distante (`SpawnTransientEntity` / `DynamicEntitySystem`),
   - id connu → **mettre à jour** sa pose (téléport/interpolation entre snapshots),
   - id disparu de snapshots successifs → **despawn** (le joueur s'est déconnecté côté serveur).

## Branchement sur le client Cyberverse (couture)

Le port **remplace** la couche zpp_bits par FlatBuffers, à un point unique et isolé :
- `EnqueueMessage(...)` → encoder un `ClientEnvelope` FlatBuffers au lieu d'un `MessageFrame` zpp.
- `PollIncomingMessages()` → décoder un `ServerEnvelope` (au lieu du `switch(frame.message_type)`),
  puis router `Snapshot` vers la logique spawn/update/despawn ci-dessus.
- **Réutiliser tel quel** : la connexion GNS (`ConnectByIPAddress`), la boucle dans le tick
  `IGameSystem` (`NetworkGameSystem::OnRegisterUpdates`), et les helpers RTTI de `Utils.h`.

## Hors-scope de cette tranche (à NE PAS implémenter maintenant — YAGNI)

- Message `Welcome{ your_id }` : inutile tant que le serveur personnalise le snapshot (exclut soi).
  À ajouter quand la **prédiction client / réconciliation** en aura besoin (Phase 1+).
- `display_name` n'est pas encore stocké côté serveur (`server_loop.rs`, TODO Phase 1).
- AoI spatiale, canaux différenciés, montage/équipement/tir (présents dans Cyberverse, hors slice).

## Critère de réussite 0-D

Deux instances du jeu (ou jeu + client de test) connectées au serveur Rust : **chaque client voit
l'avatar de l'autre se déplacer** en temps réel. C'est l'équivalent en jeu du test serveur
`two_clients_see_each_other_move`.
