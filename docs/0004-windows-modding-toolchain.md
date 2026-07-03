# ADR 0004 : Chaîne de modding Windows pour le baseline 2.31

- **Statut :** accepté
- **Date :** 2026-06-27

## Contexte

Avant de construire le client-mod, il faut une base de modding **qui démarre et tourne**
sur la version de jeu épinglée **2.31** (ADR 0001). Les loaders de modding cassent
souvent à chaque patch du jeu ; il fallait donc figer un quadruplet de versions
**réellement compatible 2.31** et le valider en jeu.

## Décision

Chaîne de référence **gelée** pour le baseline 2.31 (vérifiée sur les pages de releases
officielles le 2026-06-27, puis validée en jeu) :

| Outil | Version | Rôle |
|-------|---------|------|
| **RED4ext** | **1.30.0** | Loader natif (REDengine 4). Base dont tout dépend. |
| **redscript** | **0.5.31** (stable) | Compilateur de scripts. NE PAS utiliser les `1.0.0-preview`. |
| **Codeware** | **1.20.3** | Bibliothèque runtime (plugin RED4ext). |
| **Cyber Engine Tweaks (CET)** | **1.37.1** | Console/overlay Lua (release liée explicitement à « Game Version 2.31 »). |

- **Jeu** : Cyberpunk 2077 **2.31** sur **GOG** (+ Phantom Liberty).
- **Installation** : chaque archive est un overlay extrait à la **racine** du jeu, dans
  l'ordre **RED4ext → redscript → Codeware → CET**. Procédure détaillée :
  `client-mod/INSTALL-toolchain-2.31.md`.
- **Prérequis** : Visual C++ Redistributable 2022 (x64).

## Validation (baseline vert)

Sur machine de test (GOG 2.31, 2026-06-27) :
- L'**overlay CET s'ouvre** en jeu (touche configurée : Inser), **aucun crash**.
- Logs propres : `red4ext\logs\red4ext.log` (Codeware chargé),
  `r6\logs\redscript_rCURRENT.log` (compilation OK),
  `bin\x64\plugins\cyber_engine_tweaks\cyber_engine_tweaks.log`.

→ **Baseline « jeu moddé 2.31 » validé.** C'est la base sur laquelle se construit le
client-mod.

## Conséquences

**Positives.**
- Cible de modding stable et reproductible : quiconque installe ce quadruplet exact
  retrouve le baseline validé — indispensable pour diagnostiquer les problèmes et pour
  automatiser l'installation côté joueur.
- Découple le risque « patch de jeu » : GOG permet de rester en 2.31 sans mise à jour
  forcée.

**Négatives / risques.**
- Tout patch ultérieur du jeu (2.4x) invaliderait potentiellement ce quadruplet →
  re-pin + re-test.
- redscript 0.5.31 et Codeware 1.20.3 ne nomment pas explicitement « 2.31 » sur leurs
  releases ; la validation en jeu (ci-dessus) fait foi.

## Alternatives considérées

- **Dernières versions « tout court »** : écartées — CET est lié à **une** version de jeu
  précise (1.37.x = 2.31) ; prendre au hasard casse le baseline.
- **redscript 1.0.0-preview** : écartée — marquée « developers only », instable.
- **Installer via Vortex/MO2** : écarté — l'extraction manuelle à la racine est plus
  transparente pour diagnostiquer, et correspond au modèle overlay retenu pour
  l'installation.
