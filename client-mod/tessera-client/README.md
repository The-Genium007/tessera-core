# tessera-client — plugin RED4ext minimal (preuve de chaîne de livraison)

Plugin natif minuscule pour **Cyberpunk 2077 2.31**. Il ne fait **rien de réseau** : il se
charge et écrit une ligne de log. Son but unique est de **valider toute la chaîne de livraison
client** avant d'y verser le vrai port Cyberverse :

```
cloud-build Windows  →  emballage modset signé  →  install par le launcher  →  chargement en jeu
```

C'est le pendant côté client du `probe` côté serveur : un artefact réel et minimal qui prouve le
tuyau de bout en bout, isolément de la logique métier.

## Ce qu'il fait, concrètement

Au lancement du jeu, le loader RED4ext charge `TesseraClient.dll` et appelle son `Main`, qui écrit :

```
[info] Tessera chargé — plugin client minimal v0.1.0. La chaîne de livraison fonctionne.
```

dans `red4ext\logs\TesseraClient-<horodatage>.log` (sous la racine du jeu). Voir ce fichier après
un lancement = **preuve que le plugin livré par le launcher s'est bien chargé sur 2.31**.

## Désossage natif (phase 1, log-only — 2026-07-07)

5 catégories que le désossage redscript (`tessera-desossage/`) ne peut pas atteindre (cyberpsychos,
hustles NCPD, gigs/donneurs de quête par proximité, événements aléatoires, PNJ statiques
"community") sont exécutées nativement via un nœud de graphe de quête. Ce plugin hooke le point
d'accroche identifié (`QuestPhaseInstance::ExecuteNode`, hash AddressLib `3227858325`) en
**log-only** — aucun blocage : la sémantique du retour/des sockets de sortie de cette fonction
n'est pas encore comprise, un blocage prématuré risquerait de geler des graphes de quête. Voir le
commentaire complet dans `src/main.cpp`. **PIN IN-GAME : jamais testé.** Objectif du test : vérifier
en jeu que le hook s'attache sans erreur, puis observer les logs `[Tessera/DesossageNative]` pour
voir quand/à quelle fréquence des nœuds `questSpawnManagerNodeDefinition` passent par `ExecuteNode`
— avant d'envisager une phase 2 (blocage réel).

## Build

Compilé en `.dll` **dans le cloud** (GitHub Actions, runner `windows-latest`) — voir
`.github/workflows/client-plugin.yml`. Aucune machine Windows locale n'est requise pour *compiler* ;
seul *tester en jeu* demande le PC Windows.

Build local (optionnel, Windows + VS 2022) :

```bat
cmake -S client-mod/tessera-client -B build -A x64
cmake --build build --config Release
:: → build\Release\TesseraClient.dll
```

## Où il s'installe

Overlay déversé à la racine du jeu :

```
<racine Cyberpunk>/red4ext/plugins/TesseraClient/TesseraClient.dll
```

C'est ce que `tessera-release` emballe en package `core` du modset, que le launcher installe.

## Toolchain (ADR 0004)

RED4ext **1.30.0**, jeu **2.31**. Le SDK est épinglé à `v1.0.0` (API v1) via FetchContent dans
`CMakeLists.txt`. Le loader 1.30.0 a retiré le contrôle de version max de SDK → il charge les
plugins compilés avec un SDK récent.
