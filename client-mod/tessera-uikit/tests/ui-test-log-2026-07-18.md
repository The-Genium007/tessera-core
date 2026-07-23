# Log de test UI — florilège (palier H1)

Session 2026-07-18. Protocole complet :
`docs/superpowers/specs/2026-07-14-ui-test-harness-design.md` Partie 4
(D-H2).

| # | Écran | Commande CET | Stratégie D-U12 tentée | Résultat | Capture | Notes |
|---|---|---|---|---|---|---|
| 1 | Téléphone de V | `Tessera_UiTest("phone", true)` | Aucune (stub) | Dispatch OK, pas d'affichage | — | Log confirmé : `phone → open=true (STUB, voir commentaire du fichier pour les pistes RTTI)` |
| 2 | Radio véhicule | `Tessera_UiTest("radio", true)` | Aucune (stub) | Dispatch OK, pas d'affichage | — | Comportement stub identique |
| 3 | Radial menu | `Tessera_UiTest("radial", true)` | Aucune (stub) | Dispatch OK, pas d'affichage | — | Comportement stub identique |
| 4 | Walkie-talkie | `Tessera_UiTest("walkie", true)` | Aucune (stub) | Dispatch OK, pas d'affichage | — | Comportement stub identique |
| 5 | Console dev | `Tessera_UiTest("devconsole", true)` | Aucune (stub) | Dispatch OK, pas d'affichage | — | Comportement stub identique |
| 6 | Kitchen sink | `Tessera_UiTest("kitchensink", true)` | Aucune (stub) | Dispatch OK, pas d'affichage | — | Comportement stub identique |

**Test bonus (nom invalide)** : `Tessera_UiTest("azerty", true)` → message
fallback confirmé (`nom d'écran inconnu "azerty" — valides : phone, radio,
radial, walkie, devconsole, kitchensink`).

## Verdict palier H0

Dispatcheur `Tessera_UiTest` (`UiTestConsole.reds`) validé en jeu pour les
6 noms d'écran + le cas d'erreur. Console CET accessible via `` ` `` en
jeu, onglet Console. Les logs `[Tessera/UiTest]` apparaissent dans le
panneau de log de l'overlay CET (pas dans le champ de saisie), et dans
`cyber_engine_tweaks.log`.

**H1 reste entièrement à faire** : chaque écran est un stub qui ne fait que
logger — aucune interface ne s'affiche encore. Prochaine étape : implémenter
une vraie ouverture pour au moins un écran (candidat le plus prometteur :
`radial`, via `RadialMenuGameController.SetVisible(Bool)`), inconnue
commune à résoudre = récupérer une instance vivante du contrôleur HUD
depuis la console CET.
