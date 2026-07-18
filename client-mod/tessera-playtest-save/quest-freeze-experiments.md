# Gel de campagne — expériences journal (sonde live CET)

**Date :** 2026-07-18
**Statut :** protocole prêt, essais en jeu à faire (PC Windows)
**Portée :** save master `TesseraPlaytest` (`overlay/_tessera_playtest_save/TesseraPlaytest/`)
**Plateforme :** essais = **Windows-only** (console CET, jeu chargé). Protocole écrit sur macOS.

## But

Neutraliser la quête principale active de la save master **sans éditer le binaire** ni le graphe de
quête (D3), en passant l'entrée de journal de `q101` à l'état `Inactive` au runtime. Si ça mord et
que le jeu reste stable → figer le levier en hook au boot (style `DesossageEvents.reds`). Si ça
casse → reload, et le gel remonte au Palier 2 (blocage de nœud C++).

C'est une **mini-Gate hands-on**, ciblée sur un seul levier réversible (cf. spec
`docs/superpowers/specs/2026-07-16-desossage-campagne-gelee-observation-design.md`).

## État de départ (métadonnée committée — vérifié)

La save n'est PAS « fin Acte 2 » comme le disait la doc : c'est le **tout début de l'Acte 2**.

| Champ (`metadata.9.json`) | Valeur |
|---|---|
| `trackedQuestEntry` | `quests/main_quest/act_01/q101_resurrection/base/prepare_before_leave` |
| `debugString` | `q101_v_room` |
| `finishedQuests` | `q000 q001 q003 q004 q005` (tout le prologue) |
| `activeQuests` | `6B577A51814903CA;E118A84B9942C8C4` (2 hash 64 bits) |
| `buildPatch` / `saveVersion` | `2.31` / `269` |

Interprétation : V vient de se réveiller dans son appart post-*Heist*, `q101` en cours, avant de
sortir pour la première fois → c'est l'entrée de l'arc Johnny/relic/Takemura. **Cible = `q101`.**

## API vérifiée (dump RTTI `tools/nativedb`, 2026-07-18)

Source de vérité = le dump local + le script décompilé si on baked le hook ensuite. Ne PAS inventer
de chemin/symbole ; ce qui suit est confirmé au dump :

- `GameInstance.GetJournalManager(game)` → `gameJournalManager` (getter présent sur `ScriptGameInstance`)
- `gameJournalManager.ChangeEntryState(String, String, gameJournalEntryState, gameJournalNotifyOption) -> Bool`
  (renvoie le succès → se diagnostique tout seul)
- `gameJournalManager.ChangeEntryStateByHash(Uint32, gameJournalEntryState, gameJournalNotifyOption) -> Void`
  (⚠️ hash **Uint32** — les hash `activeQuests` de la métadonnée sont 64 bits, espace différent :
  privilégier la variante par chemin ci-dessous, pas par hash)
- Observation : `GetTrackedEntry() -> whandle`, `GetQuests(...)`, `GetEntryState(whandle) -> gameJournalEntryState`
- Enum `gameJournalEntryState` : `Undefined · Inactive · Active · Succeeded · Failed`
  → **`Inactive` = geler sans marquer réussie.** NE PAS mettre `Succeeded` (déclencherait la phase suivante).
- Enum `gameJournalNotifyOption` : `Undefined · DoNotNotify · Notify`

## Sécurité / rollback

- Filet 1 : version git de la save (`git checkout -- .../TesseraPlaytest/`).
- Filet 2 : copie externe de Lucas.
- La sonde est **live et réversible** : elle ne réécrit rien sur disque tant qu'on ne sauvegarde
  pas en jeu. Un reload de la save annule tout essai. **Ne PAS sauvegarder en jeu pendant les essais.**

## Protocole d'essai (console CET, save chargée)

> `returns true ≠ marche en jeu` : le `Bool` dit juste que l'appel a trouvé une cible. L'effet réel
> (Takemura muet, objectif disparu, jeu stable) doit être **observé**, pas déduit du retour.

**Essai 1 — geler q101 par chemin :**
```lua
local jm = Game.GetJournalManager()
local ok = jm:ChangeEntryState("quests/main_quest/act_01/q101_resurrection", "",
             gameJournalEntryState.Inactive, gameJournalNotifyOption.Notify)
print("q101 -> Inactive : " .. tostring(ok))
```
Le 2ᵉ argument (chemin d'objectif/phase) est incertain — commencer par `""`. Si `ok == false`,
itérer sur le chemin (essais 1b/1c) avant de conclure.

**Essai 1b — chemin plus précis (si 1 renvoie false) :**
```lua
-- inclure le sous-chemin vu dans trackedQuestEntry :
jm:ChangeEntryState("quests/main_quest/act_01/q101_resurrection", "base/prepare_before_leave",
  gameJournalEntryState.Inactive, gameJournalNotifyOption.Notify)
```

**Essai 2 — LIRE l'entrée réellement suivie (ne plus deviner le chemin).** Après le `false` de
l'essai 1, on introspecte l'entrée vivante pour récupérer son vrai id + hash (méthodes vérifiées au
dump : `GetTrackedEntry() -> whandle`, `entry:GetId() -> String`, `GetEntryHash(entry) -> Int32`,
`GetEntryState(entry) -> gameJournalEntryState`) :
```lua
local jm = Game.GetJournalManager()
local e = jm:GetTrackedEntry()
if e == nil then
  print("tracked entry = nil (aucune entrée suivie)")
else
  print("id    = " .. tostring(e:GetId()))
  print("hash  = " .. tostring(jm:GetEntryHash(e)))
  print("state = " .. tostring(jm:GetEntryState(e)))
end
```

**Essai 3 — geler par hash** (une fois le hash connu via l'essai 2 ; `ChangeEntryStateByHash(Uint32,
state, notify) -> Void`) :
```lua
local jm = Game.GetJournalManager()
local e = jm:GetTrackedEntry()
local h = jm:GetEntryHash(e)
jm:ChangeEntryStateByHash(h, gameJournalEntryState.Inactive, gameJournalNotifyOption.Notify)
print("nouvel état = " .. tostring(jm:GetEntryState(e)))   -- doit afficher Inactive si ça a mordu
```

> ⚠️ **Hypothèse à tester, pas acquise :** le journal est l'affichage *aval* de la quête. Passer
> une entrée à `Inactive` peut juste la **cacher du HUD** sans stopper la **phase de quête** qui,
> elle, déclenche l'appel de Takemura (système `questPhaseInstance`, en amont). Si l'essai 3 met
> bien l'état à `Inactive` **mais que Takemura appelle quand même** → c'est la preuve empirique que
> le gel doit se faire au niveau nœud (Palier 2 C++), pas au journal. C'est un résultat utile, pas
> un échec.

**Observation après un `ok == true` (marcher jusqu'à la porte de l'appart, sortir) :**
1. Takemura appelle-t-il encore ? (canal principal)
2. L'objectif de quête a-t-il disparu du HUD/journal ?
3. Le jeu reste-t-il stable (pas de freeze, pas d'écran noir, chargement OK) ?
4. La ville est-elle toujours pleinement accessible (fast-travel, districts) ?

## Résultats

| Date | Commande / essai | Retour | Observation en jeu | Verdict |
|---|---|---|---|---|
| 2026-07-18 | Essai 1 (chemin `""`) | `false` | — | chemin quête ≠ chemin journal → deviner inutile |
| 2026-07-18 | Essai 2 (lire tracked entry) | `id=call_takemura` · `hash=-1979802138` · `state=Active(2)` | — | l'entrée suivie EST l'appel Takemura → cible exacte trouvée |
| 2026-07-18 | Essai 3 (geler par hash → Inactive) | état → `Inactive (1)` confirmé | **Takemura appelle quand même** ; l'objectif se ré-affiche ; jeu stable | ❌ journal INSUFFISANT — la phase de quête amont re-déclenche. Levier journal éliminé. |

> Note console : le collage multi-lignes dans CET peut **écraser les retours à la ligne** et coller
> `end` au mot suivant (`endjm` → `'end' expected`). Parade : commande en **une seule ligne**, `;`
> entre instructions, et conversion unsigned par `% 4294967296` plutôt qu'un bloc `if/end`.

## Conclusion (2026-07-18)

**Le levier journal est éliminé.** `call_takemura → Inactive` réussit côté journal (état confirmé
`Inactive (1)`) mais **ne coupe rien** : Takemura appelle quand même, l'objectif se ré-affiche, le
jeu reste stable. Le journal est l'**affichage aval** d'une phase de quête qui, elle, re-déclenche
en amont. Confirme empiriquement la conclusion de la spec campagne-gelée (D2 / Palier 2) : le gel
doit se faire **au niveau nœud de quête**, pas au journal ni par une édition de save.

**Restait un dernier levier script possible avant le C++ :** `PhoneManager
.ApplyPhoneCallRestriction(true)` (symbole déjà éprouvé en jeu ici, cf. `DesossageEvents.reds`).
Voir Essai 4 ci-dessous.

- **Si Essai 4 coupe l'appel** → levier redscript, pas de C++ pour Takemura (mais les autres
  déclencheurs de campagne restent au Palier 2).
- **Si Essai 4 ne coupe pas** → tous les leviers script sont épuisés → **Palier 2 : hook C++
  `QuestPhaseInstance::ExecuteNode`** (`tessera-client/src/main.cpp`, fork Cyberverse), le vrai
  chantier. La save q101 reste la bonne base (premier point ville pleinement ouverte).

## Essai 4 — restriction d'appels (dernier levier script)

**Recharger la save d'abord** (l'Essai 3 a laissé `call_takemura` à `Inactive` ; on repart propre,
dans `v_room`, avant de sortir). Puis, une seule ligne en console :
```lua
Game.GetPhoneManager():ApplyPhoneCallRestriction(true); print("phone restriction ON")
```
Puis sortir de l'appart et observer : Takemura appelle-t-il encore ? (Story-call scriptée : elle
peut être un nœud `questPhoneCall` qui **contourne** la restriction ambiante — probable, mais 30 s
pour trancher.)

| Date | Commande / essai | Retour | Observation en jeu | Verdict |
|---|---|---|---|---|
| 2026-07-18 | Essai 4 (`ApplyPhoneCallRestriction(true)`) | OK | **Takemura appelle quand même** | ❌ la story-call scriptée contourne la restriction ambiante. Dernier levier script épuisé. |

## Verdict final (2026-07-18)

Les **4 essais** convergent : aucun levier accessible en redscript/CET (journal, restriction
d'appels) ne coupe l'appel Takemura. Il est déclenché par un **nœud de phase de quête** exécuté au
runtime, hors de portée du scripting. → **Palier 2 obligatoire : hook C++ `QuestPhaseInstance
::ExecuteNode`**, avec d'abord le Gate d'observation pour **identifier nommément le nœud Takemura**,
puis un blocage sélectif. Save `q101` conservée comme base. Le levier journal
(`ChangeEntryStateByHash`) reste néanmoins utile comme **nettoyage cosmétique** (cacher l'entrée du
HUD) en complément d'un blocage réel amont, jamais seul.

## Suite concrète — l'instrument est déjà prêt

Le hook C++ `ExecuteNode` **existe et journalise déjà tous les nœuds** (log-only, dédup par classe)
— `tessera-core/client-mod/tessera-client/src/main.cpp`. Le protocole de session existe aussi
(`tessera-core/client-mod/tessera-desossage/gate-observation-protocol.md`, plan
`docs/superpowers/plans/2026-07-16-desossage-gate-observation.md`). Il ne reste qu'à **le faire
tourner en jeu**.

**Ce que ces 4 essais apportent au Gate : la cible nommée.** On sait maintenant que l'entrée suivie
au moment de l'appel est **`call_takemura`** (hash `-1979802138`). Lors de la session Gate, il suffit
donc de poser un repère « Takemura appelle » à cet instant et de lire les lignes
`[Tessera/Gate/Node] classe=...` juste avant → c'est **le nom de classe du nœud à bloquer** au
Palier 2. Sans cette expérience, on cherchait à l'aveugle *quoi* observer ; on sait désormais
exactement.

**Prochain pas** (boucle PC plus lourde : build modset → deploy launcher → jouer) : emballer un
modset dev avec `tessera-client` + `tessera-desossage`, charger `q101`, provoquer l'appel, relever
la classe de nœud dans `red4ext.log`. Ensuite seulement s'écrit le blocage sélectif (Palier 2).
