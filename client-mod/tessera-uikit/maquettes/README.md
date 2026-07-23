# Maquettes HUD — kit et workflow

Dossier de **conception visuelle** des écrans HUD/UI de Tessera. On maquette en **HTML/CSS** (que je
génère et que tu ouvres dans un navigateur, sur Mac) pour décider du look **avant** de coder en
redscript ink. Le kit fixe un langage visuel commun pour que toutes les maquettes soient cohérentes.

## Les quatre fichiers

- **`catalogue-ui-natif.html`** — **LES BRIQUES : ce que le moteur accepte.** Le vocabulaire *natif*
  (widgets, contrôleurs) + la *charte graphique du jeu* (styles, atlas, police, couleurs), qu'on
  **réutilise plutôt que d'inventer**. Chaque bloc annoté avec sa vraie classe (RTTI) et sa vraie
  ressource (depot path).
- **`ecrans-natifs.html`** — **LES ÉCRANS ASSEMBLÉS : carte des écrans de base du jeu.** Pour chaque
  écran natif (inventaire, carte, téléphone, vendeur, menu pause, radial, mort, logement…), un
  **schéma de disposition** + son **contrôleur réel** (RTTI) + la **stratégie de reprise**
  (1a réinvoquer / 1b patcher / 2 reconstruire). Schémas de dev, **pas d'assets CDPR extraits**.
- **`ecrans-reconstruits.html`** — **LES ÉCRANS RECONSTRUITS EN HTML.** La même liste d'écrans natifs,
  mais reconstruits aussi fidèlement que possible à la charte du jeu (pas des wireframes : du rendu).
  Reconstructions CSS originales, contenu de démonstration inventé, **aucun asset CDPR** — sert de
  référence de design pour la reconstruction en ink.
- **`tessera-hud-kit.html`** — **NOTRE CIBLE.** Maquette de *nos* écrans (vie, faim/soif…) dans cette
  charte. Pour la phase design de nos écrans, après.
- **`maquette-lobby-arrivee.html`** — **MAQUETTE D'ÉCRAN : lobby d'arrivée** (choix du personnage).
  Vignette rouge native, fond façon inventaire, cartes de personnages + carte « Créer » (→ creator
  natif modifié), bouton CONNEXION activé à la sélection (cliquable en démo).

Ouvre-les dans un navigateur. La police Rajdhani se charge depuis Google Fonts (internet requis ;
repli condensé sinon).

## Vocabulaire natif — référence rapide (dump RTTI + Codeware)

**Widgets d'affichage :** `inkTextWidget`, `inkRichTextBoxWidget`, `inkTextInputWidget`,
`inkImageWidget`, `inkVideoWidget`, `inkVectorGraphicWidget`, `inkRectangleWidget`, `inkBorderWidget`,
`inkCircleWidget`, `inkGradientWidget`, `inkLinePatternWidget`, `inkMaskWidget`, `inkQuadShapeWidget`.

**Layouts (compound) :** `inkCanvasWidget`, `inkFlexWidget`, `inkHorizontalPanelWidget`,
`inkVerticalPanelWidget`, `inkGridWidget`, `inkUniformGridWidget`, `inkScrollAreaWidget`.

**Contrôleurs interactifs :** `inkButtonController` (+`Animated`/`Tint`/`DpadSupported`),
`inkToggleController`, `inkSliderController`, `inkListController`/`inkListItemController`,
`inkRadioGroupController`, `inkSelectorController`, `inkScrollController`.

**Animations :** `inkAnimTransparency`/`Color`/`Scale`/`Translation`, `inkTextKiroshiAnimationController`
(glitch), `inkTextReplaceAnimationController`, `inkTextValueProgressAnimationController` (compteur).

**Ressources graphiques du jeu (à référencer, jamais copier) :**
- Police : `base\gameplay\gui\fonts\raj\raj.inkfontfamily`
- Styles : `...\common\main_colors.inkstyle` (`MainColors.Blue/Red/MildRed/White/PanelBlue/PanelRed/`
  `ReadableMedium/Fullscreen_PrimaryBackgroundDarkest/…`), `dialogs_popups.inkstyle`,
  `fullscreen_main_colors.inkstyle`, `hub_menu_style.inkstyle`, `perks_style.inkstyle`
- Atlas : `...\shapes\atlas_shapes_sync.inkatlas` (parts `cell_bg`/`cell_fg`/`Plate_main`/`frame_gradient1`),
  `atlas_common.inkatlas` (icônes), `icons_keyboard.inkatlas`, `masks.inkatlas`, `shadow_blobs.inkatlas`,
  `notifications\notification_assets.inkatlas`, `notifications\vignette.inkatlas`,
  `scanning\scanner_tooltip\atlas_scanner.inkatlas`, `inventory\atlas_inventory.inkatlas`,
  `hub_menu\hub_atlas.inkatlas`
- Widget prêt : `common\buttonhints.inkwidget`

## Ce que je peux générer (esthétiquement)

- **Des maquettes HTML/CSS fidèles à l'esthétique Cyberpunk** : cadres chanfreinés (clip-path),
  scanlines, jaune #FCEE0A / cyan #00E5FF signature, typo condensée Rajdhani + mono pour les
  chiffres. Statiques ou avec un peu d'interactivité (hover, états).
- **Vite, et en itérant** : je pars des tokens du kit → chaque nouvelle maquette est cohérente avec
  les autres et avec le panneau H2 déjà validé en jeu.
- **Plusieurs variantes d'un même écran** côte à côte pour comparer.
- **La traduction ensuite en redscript ink** : les couleurs/polices du kit correspondent à ce qu'on
  sait faire côté Codeware (`SetTintColor`, `SetFontFamily raj`, `clip`/nine-slice), donc une maquette
  validée devient une spec d'implémentation directe.
- **Publication en Artifact** si tu veux un lien à ouvrir/partager sans fichier — demande-le.

## Ce que je ne peux PAS (honnêteté)

- **Pas de rendu ink réel** : une maquette HTML est un **proxy de design**, pas le jeu. Le pixel-perfect
  final se vérifie en jeu (Windows). Certaines choses (nine-slice exact d'un atlas natif, animations
  ink) s'approchent en CSS mais ne sont pas identiques.
- **Aucun asset CDPR** : on n'extrait/redistribue rien du jeu. Le kit est 100 % reconstruction CSS
  (police libre Rajdhani). Réutiliser un vrai atlas natif se fait **en jeu** via son depot path
  (`SetAtlasResource`), jamais en copiant le fichier.
- **Pas de HUD interactif jouable** ici : c'est de la maquette, l'input réel vit dans le jeu.

## Workflow

1. **Tu décris** (ou croques) un écran : « le HUD de mort », « la roue radiale à 3 étages »,
   « l'inventaire refondu »…
2. **Je génère une maquette** HTML avec les tokens du kit (souvent 2-3 variantes).
3. **Tu ouvres sur Mac, tu réagis**, on itère jusqu'à figer.
4. **Je traduis en redscript ink** sur le socle H2 (Codeware `InGamePopup`/factories), on compile-check
   en local (scc, sans lancer le jeu) puis on teste en jeu.

## Lien avec l'inventaire

Chaque écran vient de l'inventaire figé : `docs/superpowers/plans/2026-07-22-inventaire-ecrans-ui.md`
(Core d'abord). On maquette dans l'ordre de priorité décidé là-bas.
