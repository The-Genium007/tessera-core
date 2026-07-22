# Log de test UI — palier H2 (premier panneau reconstruit)

**Date :** 2026-07-22
**Palier :** H2 (spec `docs/superpowers/specs/2026-07-18-uikit-reconstruction-h2-design.md`).
**Objectif :** prouver le pipeline de reconstruction `ink` de bout en bout — une touche fait
apparaître un panneau custom visible, stylé Cyberpunk, avec curseur et clics fonctionnels.
**Environnement :** Windows, jeu GOG v2.31, Codeware 1.20.3, CET, modset dev10. Test solo, à pied.

## Ce qui a été construit

- `Tessera/uikit/UiKitDemoPopup.reds` — panneau kitchen-sink : sous-classe de `InGamePopup`
  (Codeware) → accroche écran, vignette/cadre, curseur, input modal, ESC, flou gérés nativement.
  Contenu : titre (inkText, police raj, bleu), sous-titre, rangée de 3 `SimpleButton` animés, ligne
  d'état réécrite au clic. `UseCursor() -> true`.
- `Tessera/uikit/UiKitDemoBridge.reds` — capture un `inkGameController` vivant via
  `@wrapMethod(gameuiInGameMenuGameController) RegisterInputListenersForPlayer` (pattern InkPlayground)
  + `@addMethod(PlayerPuppet) Tessera_ShowUiKitDemo()`.
- `TesseraUiKit/init.lua` (CET) — hotkey bindable « Tessera UiKit : panneau démo » → appelle le pont.

## Vérification compilation (sans lancer le jeu)

`scc.exe` standalone (recette : mémoire `redscript-local-compile-check`) → `Compilation complete`,
0 erreur, contre le vrai Codeware. Confirmé ensuite au boot réel (`redscript_rCURRENT.log` :
`Compilation complete`, 13:42:36).

## Résultat en jeu

| Critère | Résultat |
|---|---|
| Panneau apparaît à l'appui de la touche | ✅ |
| Look natif (cadre/vignette popup Cyberpunk) | ✅ |
| Titre + sous-titre + 3 boutons rendus, stylés | ✅ |
| Curseur souris affiché | ✅ |
| Survol (hover) d'un bouton → état visuel (fill rouge) | ✅ (ALPHA au survol) |
| Clic → ligne d'état mise à jour (`Dernier clic : btn_gamma`) | ✅ |
| ESC ferme le panneau | ✅ |

**Verdict : H2 réussi.** Le pipeline de reconstruction (factories ink + accroche écran via
`InGamePopup` + toggle sur touche + curseur/clic) est prouvé de bout en bout. Capture :
`01-h2-panneau-demo.png` (à déposer dans ce dossier).

## Réserves / suite

- **Sliders/toggles NATIFS fonctionnels** non couverts : ils exigent un `.inkwidget` (library
  resource) puis `inkWidgetRef.GetControllerByType(...)`, pas un `new inkSliderController()` (constat
  recoupé sur mod_settings). Différé — pas nécessaire pour prouver le pipeline. À traiter quand un
  écran palier 2 en aura besoin.
- **Suite** : les écrans précis du palier 2 (retour combat/PV/heat, lobby d'arrivée, nametags…) —
  chacun devient un assemblage de factories sur ce socle. Design visuel d'abord, puis implémentation.
