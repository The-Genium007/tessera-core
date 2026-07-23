# Direction artistique UI Tessera — LA référence

**Validée par Lucas le 2026-07-22** sur la maquette lobby v2 (`maquettes/maquette-lobby-arrivee.html`),
calibrée sur les captures des menus réels du jeu (`maquettes/capture HUD/`). **S'applique PARTOUT** :
toute nouvelle maquette ET tout écran implémenté en ink doit dériver de ces règles. En ink, on
référence les ressources natives équivalentes (`main_colors.inkstyle`, `raj.inkfontfamily`, atlas)
plutôt que des valeurs en dur quand l'équivalent existe.

## 1. Ambiance & rôles des couleurs

**Rouge-noir dominant.** Le fond est presque noir teinté rouge, avec des lueurs rouges radiales.
Chaque couleur a un RÔLE strict :

| Couleur | Hex maquette | Équivalent ink | Rôle — et RIEN d'autre |
|---|---|---|---|
| Rouge | `#e5342e` | `MainColors.Red` | **Structure** : cadres, plaques de titre, vignette, hints ESC/danger, éléments inactifs |
| Cyan | `#4be3f7` | `MainColors.Blue` | **Sélection/actif, valeurs/données, CTA principal** — jamais en décor gratuit |
| Ambre | `#f5b13d` | — | Rôles/métiers, prix, jauges secondaires (faim) |
| Blanc cassé | `#eef6f8` | `MainColors.White` | Texte principal |
| Gris lisible | `#a9b6bc` | `MainColors.ReadableMedium` | Texte secondaire |
| Fond | `#0a0508` / `#050307` | `Fullscreen_PrimaryBackgroundDarkest` | Fonds d'écran/panneaux |

## 2. Typographie

- **Rajdhani** (`raj.inkfontfamily`) pour TOUT le texte UI. Titres/labels : **uppercase + letterspacing
  large** (`.2em`+), poids 600-700.
- **Share Tech Mono** (ou équivalent mono) pour les **données** : valeurs, tags, IDs, heures, binaires.
  `tabular-nums`.

## 3. Formes & composants signés

- **Coins coupés** (chanfrein ~10-14px) sur cartes, boutons, panneaux — jamais de coins arrondis.
- **Plaque de titre navigable** : cadre rouge fin, `‹ [touche] ── TITRE ── [touche] ›` + **ticks de
  position** dessous (pattern du menu cyberware natif). Navigation clavier Q/D.
- **Sélection** : cadre cyan + coin coupé + marqueur `◤` + fond cyan translucide + glow léger.
- **CTA principal** : cyan PLEIN, texte sombre, coins coupés, glow. Inactif : gris éteint.
- **Boutons secondaires** : liseré cyan sur fond sombre, hover = remplissage rouge (pattern
  SimpleButton validé H2).
- **Bandeau d'info/notice** : liseré cyan épais à gauche + icône + texte cyan uppercase (façon
  notice native « charcudoc »).
- **Hint de sortie** : `[ESC] Quitter` rouge, bas-droite.

## 4. Décor d'écran (les « codes » de fond)

- **Scanlines** légères (multiply) sur tout écran.
- **Vignette rouge** en bord pour les écrans « hors monde » (lobby, popups — `vignette.inkatlas`).
- **Grille de croix « + »** discrète dans les zones de contenu.
- **Colonnes binaires** décoratives en bord d'écran (mono, faible opacité — rouge à gauche, cyan à droite).
- **Télémétrie décorative** sur les flancs : barres fines empilées (rouge/ambre à gauche, cyan à
  droite) avec petits tags numériques inversés.

## 5. HUD en jeu (rappels de placement — captures 2026-07-22)

- **Minimap native en HAUT-DROITE — intouchée**, pas de boussole ajoutée.
- **Faim/soif** : 2 jauges verticales **à gauche de la minimap** (ambre = faim, aqua = soif).
- **Étoiles wanted : RETIRÉES du HUD** (décision 2026-07-22) — le heat serveur existe mais sans
  affichage d'étoiles.
- **Vocal : logo micro SEUL, bas-centre**, visible **uniquement pendant le push-to-talk** — pas de
  texte « canal X parle » (décision 2026-07-22).
- Vie/armure : barres natives (état-piloté serveur, spec `2026-07-22-ui-native-branchement-multi`).
- **« Roue d'action rapide »** (nom officiel du système de roues R34, émotes incluses) — touche
  provisoire **G**. Indicateur vocal PTT = plaque chanfreinée + micro + égaliseur animé (lisible),
  jamais de texte.

## 6. Application

- Toute maquette part des tokens ci-dessus (voir `maquettes/maquette-lobby-arrivee.html` comme modèle).
- Tout écran ink reprend : `raj` + `main_colors.inkstyle` (BindProperty) + atlas de formes natifs
  (`atlas_shapes_sync` : cell_bg/cell_fg) — les hex ne servent que là où aucun binding natif n'existe.
- Un joueur doit croire que l'écran **fait partie du jeu** (jamais le branding violet Tessera Synth
  in-game — mémoire « native UI identity »).
