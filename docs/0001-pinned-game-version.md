# ADR 0001 : Version de Cyberpunk 2077 à figer

- **Statut :** accepté (cible **v2.31**)
- **Date :** 2026-06-26

## Contexte

Les outils de modding (RED4ext, redscript, Codeware) et le client Cyberverse que ce projet
forke sont verrouillés à des versions précises du jeu. Chaque patch de Cyberpunk 2077 peut
casser les loaders. Il faut donc figer UNE version de jeu cible, compatible avec toute la
chaîne d'outils, et s'y tenir.

Le plan initial mentionnait cp2077-red-socket comme brique réseau possible, mais l'analyse
du dépôt Cyberverse (voir ADR 0002) montre que ce projet embarque sa propre couche réseau
C++ via GameNetworkingSockets (Valve). **cp2077-red-socket n'est donc pas un prérequis de
ce projet** et son couplage de version devient non pertinent.

Dernière version du jeu confirmée au moment de la décision : **2.31** (patch du
11 septembre 2025). Source : https://www.cyberpunk.net/en/news/51794/patch-2-31

## Décision

**Figer la version de jeu cible à Cyberpunk 2077 v2.31** (dernier patch courant) : les
joueurs restent sur la version courante, les outils de modding à jour sont utilisables,
et c'est la meilleure base long terme pour la maintenance du client-mod.

L'alternative « rester en v2.1 » (version documentée par Cyberverse) a été écartée : elle
imposerait aux joueurs un patch de 2023 (mauvaise UX, moteur daté, contenu et correctifs
manquants). La question « porter le client Cyberverse vers 2.31 ou le reconstruire » est
tranchée dans l'ADR 0005.

### Versions connues au moment de la recherche (26 juin 2026)

| Outil | Version la plus récente | Version de jeu supportée | Source |
|---|---|---|---|
| Cyberpunk 2077 | **2.31** (dernier patch connu) | — | https://www.cyberpunk.net/en/news/51794/patch-2-31 |
| RED4ext | **v1.30.0** | non confirmé — à vérifier | https://github.com/WopsS/RED4ext/releases |
| redscript | **v0.5.31** | non confirmé — à vérifier | https://github.com/jac3km4/redscript/releases |
| Codeware | **v1.20.3** | Cyberpunk 2077 v2.31 (selon README) | https://github.com/psiberx/cp2077-codeware/releases |
| Cyberverse (client forké) | dernière master (2026-05-02) | **v2.1** (documenté README) | https://github.com/TDUniverse/Cyberverse |
| cp2077-red-socket | **v0.5.0** | v2.31 | https://github.com/rayshader/cp2077-red-socket/releases |

> **Note :** cp2077-red-socket est listé pour mémoire. Il n'est **pas utilisé** par
> Cyberverse (qui utilise GameNetworkingSockets) ni par ce projet (serveur Rust dédié).
> Voir ADR 0002.

## Conséquences

- Les dernières versions des outils de modding peuvent être utilisées (tableau ci-dessus) ;
  le quadruplet exact validé en jeu est figé dans l'ADR 0004.
- Le fork Cyberverse (documenté v2.1) nécessite un port vers 2.31 — voir ADR 0005.
- Tout patch ultérieur du jeu impose de re-vérifier la compatibilité effective de la chaîne
  avant de re-figer.
- Le serveur Rust n'est pas affecté par la version du jeu (indépendant).

## Alternatives considérées

- **Toujours mettre à jour vers le dernier patch :** trop instable pour un projet de
  modding ; chaque patch peut casser RED4ext/redscript.
- **Rester en v2.1 (version Cyberverse) :** écarté — patch de 2023 imposé aux joueurs,
  dette technique croissante côté outils.
- **Cibler cp2077-red-socket (v2.21+) :** écarté — Cyberverse embarque son propre transport
  réseau (GameNetworkingSockets) et ce projet n'utilise pas cp2077-red-socket.
- **Attendre le prochain patch CDPR :** sans horizon, bloque le développement.
