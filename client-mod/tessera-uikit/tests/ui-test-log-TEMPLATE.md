# Log de test UI — florilège (palier H1)

Copier ce template en `ui-test-log-YYYY-MM-DD.md` avant chaque session de
test, un fichier par session. Protocole complet :
`docs/superpowers/specs/2026-07-14-ui-test-harness-design.md` Partie 4
(D-H2).

Captures : nommer `NN-écran.png` (ex. `01-phone.png`) pour matcher la colonne
« Capture » ci-dessous, dans le même dossier que ce fichier. Pas de capture
automatique via CET (API non confirmée, D-H2) — touche overlay OS/GOG
Galaxy.

| # | Écran | Commande CET | Stratégie D-U12 tentée | Résultat | Capture | Notes |
|---|---|---|---|---|---|---|
| 1 | Téléphone de V | `Tessera_UiTest("phone", true)` | | | | |
| 2 | Radio véhicule | `Tessera_UiTest("radio", true)` | | | | |
| 3 | Radial menu | `Tessera_UiTest("radial", true)` | | | | |
| 4 | Walkie-talkie | `Tessera_UiTest("walkie", true)` | | | | |
| 5 | Console dev | `Tessera_UiTest("devconsole", true)` | | | | |
| 6 | Kitchen sink | `Tessera_UiTest("kitchensink", true)` | | | | |

Résultat attendu : `OK natif` / `OK patché` / `cassé` / `crash`. Après la
session, partager ce fichier + les captures pour mettre à jour
`tessera-uikit/README.md` §5 avec les stratégies confirmées.
