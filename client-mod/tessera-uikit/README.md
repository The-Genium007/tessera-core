# tessera-uikit — guide de référence `ink` pour construire des écrans RP

Guide **vivant** (mis à jour au fil des vérifications en jeu) pour construire
n'importe quel écran `ink` côté Tessera — HUD MP-overlay (nametag, scoreboard,
chat) comme écrans RP lourds (magasin, téléphone, échange...). Décisions et
rationale complets dans
`docs/superpowers/specs/2026-07-04-ui-extraction-reuse-client-mod-design.md`
(D-U1 à D-U12) — ce fichier ne les répète pas, il sert de **boîte à outils**
pour piocher dedans au moment de construire un écran précis.

**Plateforme :** Windows-only (redscript compile en jeu au lancement). Se
conçoit/écrit sur macOS, se **teste en jeu**. Au 2026-07-06, aucun palier
(U0+) n'avait démarré. Depuis le 2026-07-14, un **harnais de test
exploratoire** (paliers H0/H1, spec `2026-07-14-ui-test-harness-design.md`)
existe en parallèle du chantier U0+ — voir §0 ci-dessous. U0+ (catalogue
d'atlas + réutilisation prouvée) n'a toujours pas démarré.

**Contrainte dure (CLAUDE.md) :** ne rien redistribuer des assets CDPR. Tout ce
qui suit est construit **autour** de cette contrainte — voir Partie 4 de la
spec pour le détail de la ligne rouge.

---

## Comment piocher dans ce guide

1. Tu as un écran précis à construire (ex. « écran magasin »). Regarde d'abord
   le **catalogue** (§5) : y a-t-il un écran officiel candidat déjà repéré ?
2. Applique la **hiérarchie de stratégie** (§1) dans l'ordre : réinvocation
   native verbatim → réinvocation native patchée → reconstruction → assets
   originaux. Ne descends d'un niveau que si le précédent est vraiment
   impossible.
3. Une fois la stratégie choisie, va chercher la **capacité** dont tu as besoin
   (§2 : images, texte, animation, son, interactivité...) pour le détail
   technique et l'exemple de code.
4. Mets à jour le catalogue (§5) avec ce que tu as trouvé/prouvé — comme
   `tessera-desossage/README.md` le fait pour ses leviers.

---

## 0. Harnais de test exploratoire (paliers H0/H1)

Spec dédiée : `docs/superpowers/specs/2026-07-14-ui-test-harness-design.md`
(D-H1 à D-H7). Objectif : qualifier rapidement l'affichage d'un florilège de
6 écrans candidats, avant de choisir lesquels deviennent des features
réelles — distinct du chantier U0+ (catalogue d'atlas) ci-dessous, qui n'a
pas démarré.

**Mécanisme (H0)** : `Tessera_UiTest(name: String, open: Bool)`, méthode
`@addMethod` sur `PlayerPuppet`, appelable depuis la console CET sans
rebuild — même pattern que `Tessera_SetLever`
(`tessera-desossage/README.md`).

```lua
Game.GetPlayer():Tessera_UiTest("phone", true)
Game.GetPlayer():Tessera_UiTest("phone", false)
```

Noms d'écran valides : `phone`, `radio`, `radial`, `walkie`, `devconsole`,
`kitchensink`. Fichiers :
`overlay/r6/scripts/Tessera/uikit/UiTestConsole.reds` (dispatcheur) + un
`UiTest<Écran>.reds` par écran, chacun un **stub documenté** — RTTI-vérifié
via `tools/nativedb` là où un équivalent natif existe (phone/radio/radial/
kitchensink), reconstruction pure sans recherche native pour walkie/
devconsole (D-H7) — dans les deux cas, pas encore testé en jeu (palier H1).

**Tester (H1)** : copier `tests/ui-test-log-TEMPLATE.md` en
`tests/ui-test-log-YYYY-MM-DD.md`, suivre le protocole de la spec (Partie 4,
D-H2) — un fichier log + des captures manuelles par session, partagés après
coup pour mettre à jour ce guide.

---

## 1. Hiérarchie de stratégie par écran (D-U12)

| # | Stratégie | Quand | Coût | Risque |
|---|---|---|---|---|
| 1a | Réinvocation native **verbatim** | L'écran natif fait déjà exactement ce qu'on veut | Quasi nul | Faisabilité non prouvée (recherche RTTI par écran) |
| 1b | Réinvocation native **patchée** | Même écran natif, mais il faut masquer/ajouter/réarranger de l'affichage | Faible-moyen | Dépend des noms de méthodes internes du contrôleur (RTTI, comme désossage) — fragile aux mises à jour, atténué par la version pinnée v2.31 |
| 2 | **Reconstruction** | Aucune réinvocation ne convient (comportement différent requis, ex. walkie ≠ téléphone) | Moyen-élevé (un arbre de widgets à recréer) | Faible une fois fait — c'est notre code |
| 3 | **Assets originaux** | Rien de natif ne s'y prête (banque/ATM, permis RP) | Élevé (design + conversion WolvenKit + ArchiveXL) | Faible — mais 100% à notre charge |

**Frontière stricte pour 1b** : le patch ne touche qu'à la **présentation**
(quels widgets sont visibles, comment ils sont arrangés, quel texte statique
ils affichent). Toute évolution de ce que représentent les **données**
sous-jacentes (quels perks existent, leurs effets, l'équilibrage économique
d'un magasin) est un mod de contenu/`TweakDB` — hors périmètre de
`tessera-uikit`.

### 1a/1b — Workflow réinvocation native

1. Identifier le système/contrôleur qui affiche l'écran (RTTI dump NativeDB →
   scripts décompilés → dernier recours C++ hook, cf. mémoire « native symbol
   triage » — même méthode que désossage).
2. Trouver le point d'entrée pour le déclencher **hors de son contexte
   normal** (ex. le character creator ne s'affiche normalement qu'au
   prologue — le déclencher ailleurs est la vraie inconnue technique, cf.
   `2026-07-06-character-creation-design.md` Partie 4).
3. **1a** : consommer sa sortie telle quelle (ex. capturer l'apparence en
   blob).
   **1b** : `@wrapMethod`/`@replaceMethod` sur sa méthode de construction ou de
   peuplement pour ajouter/masquer/réordonner des enfants dans l'arbre déjà
   construit par le jeu.
4. Vérifier en jeu, documenter dans le catalogue (§5) : mécanisme réel, testé
   ou non, limites.

### 2 — Workflow reconstruction

1. **WolvenKit, en local** : ouvrir le `.inkwidget` de l'écran de référence,
   l'exporter en JSON lisible (hiérarchie complète : quel panel contient quoi,
   quelles proportions, quel atlas/quelle part par icône, quel style). Ce
   fichier JSON **ne quitte jamais le PC de dev**, il n'est jamais commité.
2. Transcrire cette hiérarchie en redscript qui construit l'équivalent à
   l'exécution (`inkCanvas`/`inkVerticalPanel`/`inkImage`...), en référençant
   les **mêmes** depot paths d'atlas/style/police relevés à l'étape 1.
3. Vérifier en jeu via `UiKitProbe.reds` (commande console CET, spawn sans
   rebuild — même pattern que `DesossageConsole.reds`).
4. Une fois l'équivalent qui tourne, le modifier librement pour la variante
   voulue (ex. téléphone → walkie : remplacer icône appel par icône PTT,
   renommer les champs) — c'est désormais notre code, pas un fichier CDPR.

---

## 2. Référence par capacité

Chaque entrée : parallèle web, widget(s)/API `ink` réels, statut, limites.
Statut : **Confirmé** (retrouvé tel quel dans un mod réel publié ou la doc
communautaire) vs **À vérifier** (intention documentée, syntaxe non testée sur
notre PC).

### Layout

| Ink | Parallèle web | Rôle |
|---|---|---|
| `inkCanvas` | `<div>` position libre | conteneur libre, positionnement absolu des enfants |
| `inkHorizontalPanel` / `inkVerticalPanel` | flexbox row/column | empilement horizontal/vertical |
| `inkScrollArea` | `overflow: scroll` | zone de défilement |
| `inkBorder` / `inkRectangle` | `border` / fond coloré | décor |

**Statut :** Confirmé — vocabulaire de base documenté par le wiki communautaire
(`wiki.redmodding.org`, page « Inkwidgets: a custom interface »).

### Texte et saisie

| Ink | Parallèle web | Rôle |
|---|---|---|
| `inkText` | `<span>`/`<p>` | texte statique/dynamique |
| `inkTextInput` | `<input>` | saisie utilisateur (ex. montant à envoyer) |

**Statut :** À vérifier sur PC (les noms de propriétés exacts type binding de
police/couleur ne sont pas encore confirmés dans nos tests).

### Images (atlas)

Une `inkImage` ne charge jamais un fichier directement : elle pointe vers un
**inkatlas** (déjà chargé par le jeu) + un **nom de part**.

```reds
img.SetAtlasResource(r"base\gameplay\gui\common\icons\valency_corporations.inkatlas");
img.SetTexturePart(n"biotechnica-alt");
```

**Statut : Confirmé** — exemple retrouvé tel quel dans un mod publié (pas
seulement une hypothèse D-U6 initiale). Aucune copie d'asset : le jeu du
joueur charge sa propre texture, on ne fait que la désigner.

### Animations / effets

```reds
let anim = new inkAnimTransparency();
anim.SetStartTransparency(0.0);
anim.SetEndTransparency(1.0);
let animDef = new inkAnimDef();
animDef.AddInterpolator(anim);
// options : inkAnimOptions.loopInfinite, inkanimLoopType.Cycle...
```

**Statut : Confirmé** — pattern retrouvé dans la doc communautaire (interpolateurs
`inkAnimTransparency` et consorts + `inkAnimOptions`). Parallèle web : CSS
transitions/keyframes.

### Son

Trois façons documentées, toutes natives (pas besoin d'un widget dédié) :

```reds
// Méthode événement
let evt = new SoundPlayEvent();
evt.soundName = n"ono_v_effort_short";
player.QueueEvent(evt);

// Méthode helper
PlaySound(n"ui_menu_click"); // déclenchable depuis un handler de widget (OnClick, etc.)
```

**Statut : Confirmé** (`SoundPlayEvent`/`AudioEvent`/`PlaySound` documentés sur
`wiki.redmodding.org`, page « Playing Sounds in-game »). Pour du contrôle avancé
(panning, reverb, sons custom) : mod **Audioware**, optionnel. Parallèle web :
`<audio>`/Web Audio API.

### Vidéo

**Hors périmètre par défaut (D-U8).** Pas de widget vidéo natif dans `ink`. Le
seul chemin trouvé est un mod tiers (« Simple Video Framework ») qui dessine un
HUD redscript sur un **plane mesh** dans le monde 3D (façon écran de TV) —
sortie du cadre `ink` classique, dépendance lourde. À ne considérer que si un
écran précis l'exige vraiment (aucun cas identifié dans le catalogue §5 pour
l'instant).

### Interactivité / état

`inkWidget` a un état (`Default`/`Hover`/`Press`/`Disabled`) piloté par le
style — parallèle direct aux pseudo-classes CSS `:hover`/`:disabled`.

**Statut : À vérifier sur PC** (pattern connu du modding `ink` en général, nom
de méthode exact — probablement `SetState(inkWidgetState.Disabled)` — pas
encore confirmé par une source concrète pour ce projet).

### Verrouillage (D-U10) — 3 mécanismes distincts

| Mécanisme | Portée | Implémentation |
|---|---|---|
| Interactivité conditionnelle | widget | état `Disabled` réactif, client-only (voir ci-dessus) |
| Accès à l'écran gated serveur | écran entier | le widget ne se peuple qu'après une réponse serveur validée (protocole FlatBuffers — pas une capacité `ink`) |
| Mini-jeu visuel façon digicode/hacking | gameplay | **différé** — le plus lourd des trois, pas construit tant qu'un écran précis n'en a pas besoin |

---

## 3. Ce qu'on ne fait PAS ici

- Redistribuer un `.inkwidget`/`.inkatlas`/`.xbm` du jeu, modifié ou non.
- Changer ce que représentent des données de jeu (perks, prix, équilibrage)
  via un hook 1b — c'est un mod de contenu, pas de l'UI.
- Construire un système de vidéo en `ink` (D-U8).
- Construire le mini-jeu de verrouillage façon hacking avant qu'un écran RP
  précis en ait vraiment besoin.

---

## 4. Fichiers (prévus, pas encore créés — voir paliers U0+ de la spec)

| Fichier | Rôle |
|---|---|
| `extract/scan-atlases.md` | procédure WolvenKit pour peupler le catalogue d'atlas |
| `extract/catalog/*.json` | catalogue de métadonnées (depot paths + parts) — jamais d'assets |
| `runtime/.../UiKitAtlas.reds` | `inkImage` ← (atlasPath, partName) |
| `runtime/.../UiKitStyle.reds` | applique un `.inkstyle` existant |
| `runtime/.../UiKitFactory.reds` | fabriques de composants (nametag, scoreboard row, prompt...) |
| `runtime/.../UiKitNative.reds` | réinvocation native (1a/1b) — rappel + hooks de contrôleurs d'écran |
| `runtime/.../UiKitProbe.reds` | commande console CET pour tester un widget en jeu sans rebuild |

---

## 5. Catalogue des écrans RP — bases de reconstruction candidates

Table **vivante** — mise à jour au fil des découvertes en jeu (même logique que
« l'état des leviers » de `tessera-desossage/README.md`). Version figée au
moment de la spec dans
`2026-07-04-ui-extraction-reuse-client-mod-design.md` Partie 8.2.

| Catégorie | Écran RP visé | Base officielle candidate | Stratégie candidate | Statut |
|---|---|---|---|---|
| Communication | Téléphone (appels, contacts, textos) | `HudPhoneGameController`/`PhoneSystem` (RTTI confirmé, palier H0) | 1a à tester en premier (H1) | Repéré (harnais H0) |
| Communication | Radio véhicule (stations partagées) | `VehicleRadioPopupGameController.Activate()` (RTTI confirmé, palier H0) | 1a à tester en premier (H1) | Repéré (harnais H0) |
| Communication | Walkie-talkie / radio (canaux, PTT) | Aucune (concept 100% Tessera, D-H7) | 2 (reconstruction, pas de recherche native) | Repéré (harnais H0, stub) |
| Commerce | Magasin (achat/vente) | Écran vendeur/marchand | 1a/1b à explorer, sinon 2 | Pas exploré |
| Commerce | Distributeur / kiosque | `VendingMachineControllerPS` | 1b (déjà hooké par désossage pour autre chose) | Repéré (désossage) |
| Commerce | Échange entre joueurs | Pas d'équivalent direct | 2, inspiré du vendeur/inventaire | Pas exploré |
| Commerce | Envoi d'argent entre joueurs | Pas d'équivalent direct | 3 (composant simple) | Pas exploré |
| Commerce | Banque / ATM | Style d'un terminal de hacking | 3 (style seulement) | Pas exploré |
| Inventaire | **[PRIORITÉ]** Inventaire personnel | Écran d'inventaire | 1a/1b à explorer, sinon 2 | Pas exploré |
| Inventaire | **[PRIORITÉ]** Craft / démontage | Écran de craft/démontage | 1a/1b à explorer, sinon 2 | Pas exploré |
| Inventaire | Coffre / stockage partagé | Écran d'inventaire, variante deux-colonnes | 2 | Pas exploré |
| Identité | **[PRIORITÉ]** Fiche de personnage | Écran de caractéristiques/attributs | 1a/1b à explorer, sinon 2 | Pas exploré |
| Identité | **[PRIORITÉ]** Montée personnelle / attribution de points | Écran d'attribution (level-up, partagé avec le creator) | 1b présentation seulement — jamais la donnée | Pas exploré |
| Identité | **[PRIORITÉ]** Création de personnage | Character creator natif | 1a — **propriété de `2026-07-06-character-creation-design.md` B2, ne pas dupliquer ici** | Recherche en cours (spec dédiée) |
| Social | Menu d'interaction / roue contextuelle (+ radial imbriqué/émotes) | `RadialMenuGameController.SetVisible(Bool)` (RTTI confirmé, palier H0) | 1a à tester en premier (H1) | Repéré (harnais H0) |
| Feedback | Notifications système | Popups HUD natifs | 1a/1b à explorer, sinon 2 | Pas exploré |
| Feedback | Barre de progression / timer | Cercle breach protocol / barre de craft | 1a/1b à explorer, sinon 2 | Pas exploré |
| Navigation | Statut logement/loyer | `ApartmentScreenControllerPS` | 1a/1b déjà repéré | Repéré (désossage) |

**[PRIORITÉ]** = cluster création de personnage. U0/U1 (catalogue d'atlas +
`inkImage` référencée en jeu) sont le prérequis minimal de ce cluster — voir
note de priorité dans la spec, Partie 5.
