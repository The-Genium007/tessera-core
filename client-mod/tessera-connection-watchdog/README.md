# tessera-connection-watchdog — notifie la perte de connexion serveur

Répond à un signalement de playtest (2026-07-07) : quand le serveur Rust crash ou coupe la
connexion, rien n'avertit le joueur — le jeu continue de tourner en local comme si de rien
n'était, sans aucun retour visuel.

**Plateforme :** Windows-only (CET Lua, s'exécute au chargement du jeu). Conçu sur macOS via
lecture du fork Cyberverse, à tester en jeu.

## Comment ça marche

Le plugin natif (fork Cyberverse, `client/red4ext/src/NetworkGameSystem.cpp`,
`ConnectionStatusChangedCallback`) bascule déjà un booléen `FullyConnected` à `false` dès que l'état
GNS quitte `k_ESteamNetworkingConnectionState_Connected` — mais ce booléen n'était consulté que par
`Cyberverse.reds` (`OnSavesForLoadReady`, pour différer un chargement), jamais montré au joueur.

Plutôt que de modifier le plugin natif (aurait nécessité un rebuild + une nouvelle release
`netcode-v*`, cycle long, voir `.github/workflows/modset-release.yml`), ce mod lit `FullyConnected`
depuis CET : c'est une propriété RTTI déjà exposée et utilisée telle quelle côté redscript
(`networkSystem.FullyConnected`) — lecture attendue identique côté Lua CET, via le même pont RTTI
que `Game.GetPlayer()`/`Game.GetTeleportationFacility()` déjà confirmés fonctionnels dans ce projet.
`Game.GetNetworkGameSystem()` est un `@addMethod(GameInstance)` ajouté par le fork (pas natif CDPR),
mais exposé par RTTI comme n'importe quelle autre méthode `GameInstance`.

Chaque frame (`onUpdate`), on relit `FullyConnected` ; sur la transition `true -> false`, on affiche
une fenêtre ImGui rouge persistante ("CONNEXION AU SERVEUR PERDUE" + durée écoulée) tant que la
connexion n'est pas rétablie.

## État (2026-07-07)

- **Jamais testé en jeu.** Deux points précis à confirmer au premier lancement (voir `PIN IN-GAME`
  dans `init.lua`) :
  1. `Game.GetNetworkGameSystem()` résout bien la méthode custom du fork depuis CET ;
  2. `.FullyConnected` se lit comme un champ Lua normal sur l'objet retourné.

  Si l'un des deux échoue, `print()` explicite dans la console CET (voir la fonction
  `checkConnection`) — pas d'échec silencieux.
- Pas de logique de reconnexion automatique côté fork (le flag ne repasse à `true` que si une
  nouvelle connexion GNS s'établit) — ce mod ne fait qu'**informer**, il ne tente rien côté réseau.
- Ajouté à `distribution/modset.packages.toml` (`tessera-connection-watchdog`, `required = true`).

## Où ça se déploie

Overlay enraciné à la racine du jeu :
`<racine Cyberpunk>/bin/x64/plugins/cyber_engine_tweaks/mods/TesseraConnectionWatchdog/init.lua`.
Empaqueté dans le modset client par `tessera-release`, installé par le launcher (overlay générique).
