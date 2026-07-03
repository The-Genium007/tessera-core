# ADR 0005 : Porter le client Cyberverse vers 2.31 (vs reconstruction)

- **Statut :** proposé (à confirmer par un build Windows réel ; voir note de mise à jour
  en fin de document)
- **Date :** 2026-06-27

## Contexte

L'ADR 0002 a acté la **réutilisation de Cyberverse**. Restait à trancher, pour le
**client-mod**, entre **porter** le client Cyberverse (qui cible le jeu 2.1) vers
**2.31**, ou **reconstruire** un client neuf en s'inspirant de ses patterns.

Une analyse des sources de Cyberverse a été menée. Constats clés :
- **Couplage à la version du jeu très faible** : zéro offset/adresse en dur, tout l'accès
  jeu passe par **RTTI par nom** (via red-lib/Codeware) et les en-têtes générés de
  RED4ext.SDK.
- Surface jeu-spécifique **minuscule et concentrée** (`Utils.h`, `NetworkGameSystem.cpp`,
  `PlayerActionTracker.cpp`, `Cyberverse.reds`).
- Transport **GameNetworkingSockets** (le même que notre serveur) et **découpage en
  4 modules** identiques à notre design.
- Sérialisation **zpp_bits + protocole maison** → **diverge** de notre choix
  **FlatBuffers** ; mais la couture est isolée
  (`EnqueueMessage`/`PollIncomingMessages`).

## Décision

**Porter le client Cyberverse (RED4ext C++ + redscript) vers 2.31**, en réutilisant :
- la boucle réseau **GNS** dans un tick `IGameSystem`,
- les **hooks redscript** d'intégration gameplay,
- les patterns de **spawn/teleport/interpolation** d'entités (`DynamicEntitySystem`).

Et en **remplaçant** leur couche de sérialisation **zpp_bits/protocole maison** par
**notre protocole FlatBuffers**, branché à la couture
`EnqueueMessage`/`PollIncomingMessages`, le client se connectant à **notre serveur Rust**.

**Ne PAS réutiliser** leur serveur (C#/Native) ni leur sérialisation.

Le port consiste à : mettre à jour les submodules **RED4ext.SDK + red-lib** vers leur
version compatible **2.31**, builder (vcpkg + CMake + Ninja, Windows), puis auditer les
~19 appels RTTI et les hooks redscript susceptibles d'avoir bougé entre 2.1 et 2.31.

## Pourquoi proposé et non accepté

Deux inconnues ne pouvaient être levées que par un **build + lancement réel sur
Windows** :
1. la **disponibilité d'une build RED4ext.SDK/red-lib pour 2.31** ;
2. la **compatibilité effective des noms/signatures RTTI** et des hooks redscript en 2.31.

Si le build échoue de façon diffuse (peu probable vu l'analyse), on rebascule sur une
reconstruction ciblée en réutilisant les mêmes patterns. L'ADR passera en **accepté**
une fois le client porté chargé en jeu sans crash.

## Conséquences

**Positives.**
- Effort de port faible et localisé (pas de réécriture d'offsets) ; réutilisation
  maximale.
- Cohérence avec les décisions verrouillées du projet : on garde **serveur Rust +
  FlatBuffers**, on ne récupère que le client et les patterns d'intégration.
- La couture de sérialisation nette permet de brancher notre protocole sans réécrire la
  logique jeu.

**Négatives / risques.**
- Dépendance à l'écosystème psiberx (RED4ext.SDK/red-lib) pour le support 2.31.
- Les hooks redscript gameplay sont le point de fragilité réel à valider patch par patch.
- Mélange de licences à vérifier (Cyberverse + dépendances) avant toute redistribution
  binaire du client-mod. (Licence Cyberverse vérifiée depuis : MIT — voir addendum
  ADR 0002.)

## Alternatives considérées

- **Reconstruire le client de zéro** : écartée — le couplage version est déjà abstrait,
  la surface jeu-spécifique est minime ; reconstruire ne gagnerait quasi rien tout en
  perdant des patterns rodés.
- **Réutiliser aussi leur serveur/sérialisation** : écartée — contredit les choix serveur
  Rust et FlatBuffers, et leur protocole dupliqué-à-la-main sans versioning est un
  cul-de-sac.

## Note de mise à jour (2026-07-02)

Le ré-audit documenté dans l'addendum de l'ADR 0002 rapporte que le port vers 2.31
**compile en CI et charge en jeu sans crash** (RTTI enregistré, les ~19 appels tiennent
sur 2.31). Les deux inconnues ci-dessus sont donc levées. Reste à prouver l'échange
réseau réel avec 2+ joueurs sur 2.31 (couture FlatBuffers en place, pas encore validée
en jeu) avant de clore définitivement cette ADR.
