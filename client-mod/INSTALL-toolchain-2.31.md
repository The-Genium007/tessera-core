# Installation manuelle de la toolchain de modding — Cyberpunk 2077 **2.31**

Procédure pour installer **à la main** les mods nécessaires au baseline (ce que le launcher
automatisera plus tard). Cible : **CP2077 2.31 (GOG + Phantom Liberty)**. Plateforme : **Windows**.

> Objectif : obtenir un **jeu moddé qui démarre sans crash** avec l'overlay CET fonctionnel.
> C'est la base sur laquelle se construira le client-mod TesseraSynth.

## Étape 0 — Prérequis & sécurité

1. **Visual C++ Redistributable 2022 (x64)** installé (requis par RED4ext et CET).
2. Dans GOG Galaxy : **désactiver les mises à jour auto** du jeu (⚙️ *Manage installation → Configure → Disable auto-updates*).
3. **Sauvegarder une copie vanilla** du dossier du jeu (au moins `bin\` + `archive\`).
4. Mettre le jeu en **Windowed Borderless** et **désactiver les overlays** (Steam/Discord/GOG) — exigé par CET.
5. Repérer la **racine du jeu** (dossier contenant `bin\`, `archive\`, `REDprelauncher.exe`).
   GOG typique : `…\GOG Galaxy\Games\Cyberpunk 2077\`.

> 💡 Après téléchargement, fais **clic droit → Propriétés → Débloquer** sur chaque `.zip`
> (sinon Windows peut bloquer les DLL).

## Étape 1 — Versions à télécharger (compatibles 2.31)

| # | Mod | Version | Lien (asset, pas « source code ») |
|---|-----|---------|-----------------------------------|
| 1 | **RED4ext** | **1.30.0** | https://github.com/WopsS/RED4ext/releases/tag/v1.30.0 |
| 2 | **redscript** | **0.5.31** | https://github.com/jac3km4/redscript/releases/tag/v0.5.31 |
| 3 | **Codeware** | **1.20.3** | https://github.com/psiberx/cp2077-codeware/releases/tag/v1.20.3 |
| 4 | **Cyber Engine Tweaks** | **1.37.1** | https://github.com/maximegmd/CyberEngineTweaks/releases/tag/v1.37.1 |

> ⚠️ redscript : prendre **0.5.31** (stable), **pas** les `1.0.0-preview` (« developers only »).

## Étape 2 — Installer, dans l'ordre (chaque zip s'extrait à la **racine** du jeu)

Le principe : chaque archive est déjà structurée par rapport à la racine du jeu — tu
**extrais à la racine et tu fusionnes** les dossiers.

1. **RED4ext 1.30.0** → extraire à la racine.
   Crée `red4ext\` + un loader dans `bin\x64\`. *(RED4ext doit être installé en premier : les autres en dépendent.)*
2. **redscript 0.5.31** → extraire à la racine.
   Crée/complète `engine\` et `r6\`.
3. **Codeware 1.20.3** → extraire à la racine.
   Va dans `red4ext\plugins\Codeware\` (c'est un plugin RED4ext).
4. **Cyber Engine Tweaks 1.37.1** → extraire à la racine.
   Remplit `bin\x64\` (`version.dll`, `global.ini`, `plugins\`).

## Étape 3 — Lancer & valider (le test qui compte)

Lance le jeu **normalement** (GOG / REDprelauncher).

1. **1er démarrage CET** : une fenêtre apparaît pour **choisir la touche de l'overlay** —
   choisis-en une (ex. `Inser`, `Début`, ou `²`/`~`) et valide, puis accepte l'avertissement.
2. En jeu, **appuie sur cette touche** → l'**overlay CET (console) doit s'ouvrir**.
3. **Aucun crash**, le jeu atteint le menu principal et tourne.

### ✅ Critères de réussite (logs propres)

| Mod | Log à vérifier | Attendu |
|-----|----------------|---------|
| RED4ext | `red4ext\logs\red4ext.log` | démarrage OK, **plugin `Codeware` listé comme chargé** |
| redscript | `r6\logs\redscript_rCURRENT.log` | **compilation réussie**, pas d'erreur |
| CET | `bin\x64\plugins\cyber_engine_tweaks\cyber_engine_tweaks.log` | chargé, pas d'erreur fatale |
| CET (in-game) | — | **overlay s'ouvre** |

Si l'overlay s'ouvre et que les 3 logs sont propres → **baseline jeu moddé 2.31 validé.**

## Si ça casse

- **Crash au lancement / pas d'overlay** : 99 % du temps c'est une **incompatibilité de version**
  (un mod ne correspond pas à 2.31) ou le **VC++ Redist 2022** manquant.
- Récupère le **contenu des 3 logs** ci-dessus + le **moment du crash** (avant menu ? à l'ouverture overlay ?).
- En dernier recours, restaure la copie vanilla (Étape 0) et on réinstalle un mod à la fois pour isoler.

## Désinstaller (revenir vanilla)

Supprimer `red4ext\`, `engine\tools\`, `r6\` (les ajouts), et dans `bin\x64\` :
`version.dll`, `global.ini`, le dossier `plugins\`. Ou simplement restaurer la copie vanilla.

---

Une fois ce baseline ✅ confirmé, on gèle ces 4 versions dans un ADR et on attaque la suite
(build/port de Cyberverse, puis le client-mod TesseraSynth).
