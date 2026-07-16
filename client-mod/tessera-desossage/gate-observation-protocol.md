# Gate — protocole de session d'observation profonde

Instrument construit dans `docs/superpowers/plans/2026-07-16-desossage-gate-observation.md`
(spec : `docs/superpowers/specs/2026-07-16-desossage-campagne-gelee-observation-design.md`).
**Tout est log-only** — cette session n'a pour but que de LIRE ce qui se passe, aucun blocage
n'est actif.

## Avant de jouer

1. Emballer un modset dev incluant `tessera-desossage` (déjà `required = true`), publier sur le
   canal dev, installer via le launcher.
2. Repérer les deux fichiers de log à surveiller pendant/après la session :
   - `red4ext.log` (racine `bin/x64/`) — canal 1 (nœuds de quête), préfixe `[Tessera/Gate/Node]`
     et `[Tessera/Gate/Summary]`.
   - `bin/x64/plugins/cyber_engine_tweaks/scripting.log` — canaux 2-4 (facts, état désossage,
     actions joueur), préfixes `[Tessera/Gate/Fact]`, `[Tessera/Gate/State]`,
     `[Tessera/Gate/Action]`.
3. Copier `gate-observations-TEMPLATE.md` en `gate-observations-YYYY-MM-DD.md` (même dossier).

## Pendant la session

1. Charger la save `TesseraPlaytest`. Noter l'heure de chargement.
2. Poser un premier repère : panneau CET Désossage → champ "Repère" → écrire `"chargement
   session"` → bouton "Poser le repère" (ou console : `Game.GetPlayer():Tessera_GateMark("chargement session")`).
3. Sortir de l'appartement de V. **Dès que Takemura appelle (ou tout autre événement notable)**,
   poser immédiatement un repère décrivant ce qui vient de se passer (ex.
   `"Takemura appelle"`) — le timestamp du repère permet de corréler avec les lignes
   `[Tessera/Gate/Node]`/`[Tessera/Gate/Fact]` autour du même moment.
4. Se déplacer en ville plusieurs minutes : croiser des PNJ statiques (groupes qui discutent),
   tenter d'interagir avec un donneur de quête, chercher une rencontre/hustle spontanée. Poser un
   repère à chaque occurrence.
5. Laisser tourner au moins 10-15 minutes pour que le résumé périodique (`[Tessera/Gate/Summary]`,
   tous les 500 appels à `ExecuteNode`) ait le temps d'apparaître plusieurs fois.

## Après la session

1. Récupérer les deux fichiers de log.
2. Remplir `gate-observations-YYYY-MM-DD.md` : pour chaque repère posé, chercher dans les logs les
   lignes proches du même timestamp (`[Tessera/Gate/Node]` en priorité — c'est le canal qui
   identifie les noms de classe de nœuds).
3. Extraire la liste des noms de classe distincts vus (`classe=...` dans les lignes
   `[Tessera/Gate/Node]`) qui apparaissent juste avant/pendant un événement noté (Takemura, PNJ
   statique, hustle) — c'est la **liste nommée** qui alimentera le plan Palier 2 (blocage
   sélectif, hors périmètre de ce plan).
4. Partager le fichier rempli pour synthèse.
