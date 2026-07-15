module Tessera.Desossage

// Marqueurs carte/minimap (quêtes, vendeurs, POI, voyage rapide) + rôle PNJ (icône au-dessus de
// la tête — vendeur, donneur de mission…). Deux mécanismes RTTI distincts pour un même symptôme
// visuel côté joueur :
// - `MappinSystem` : les marqueurs globaux (monde/minimap/carte). Noms confirmés via le script
//   décompilé officiel (CDPR-Modding-Documentation/Cyberpunk-Scripts,
//   scripts/core/systems/mappinSystem.script) — redscript expose la classe/struct/ID sous un nom
//   court sans préfixe : `MappinSystem`, `MappinData`, `MappinEntry`, `NewMappinID`, `GameObject`
//   (au lieu des noms bruts du dump RTTI `gamemappinsMappinSystem`/`gamemappinsMappinEntry`/
//   `gameNewMappinID`/`gameObject`), contrairement à l'enum `gamemappinsMappinTargetType` qui,
//   elle, garde son nom RTTI complet — CASSÉ deux fois de suite avant de trouver le bon nom
//   (2026-07-05).
// - CASSÉ, RETIRÉ (2026-07-05) : `RegisterMappin`/`RegisterMappinWithObject` en `@wrapMethod` —
//   erreur `this signature does not match any existing method` malgré des types individuellement
//   valides (MappinData/NewMappinID/GameObject tous acceptés isolément). Cause exacte non
//   identifiée (visibilité ? params optionnels de RegisterMappinWithObject changés en v2.31 ?) —
//   pas re-tenté à l'aveugle après deux échecs de suite sur ce fichier. PISTE POUR PLUS TARD :
//   chercher un mod publié récent qui wrap concrètement l'une de ces deux méthodes (pas juste qui
//   les appelle, comme `tiltedphoques/CyberpunkMP` ou le sample `limitedFastTravel.reds`) pour
//   copier une signature confirmée compilable.
// - `GameplayRoleComponent` : composant attaché aux PNJ/devices avec un "rôle" gameplay ; porte le
//   marqueur au-dessus de la tête. `OnGameAttach()` confirmé déclaré directement dessus (RTTI,
//   `search.py show GameplayRoleComponent` sans --deep) — n'a jamais été signalé en erreur sur 2
//   tentatives de compilation, mais reste PIN IN-GAME côté comportement (jamais vu tourner).
//   `EGameplayRole` n'a pas de valeur "Vendor"/"QuestGiver" explicite dans le dump RTTI (juste des
//   rôles génériques type ServicePoint/StoreItems/NPC/GenericRole) — le hook masque donc TOUS les
//   rôles sans distinction, y compris les policiers en patrouille (confirmé en jeu 2026-07-05 :
//   perdent leur marqueur au-dessus de la tête dès qu'ils s'attachent, ex. à l'arrivée d'un
//   nouveau chunk). **Comportement VOULU, pas un bug** (clarifié par Lucas 2026-07-05) : l'objectif
//   du désossage est de vider la carte de TOUS les marqueurs/icônes pour reconstruire de zéro un
//   monde compatible multijoueur — la portée large de ce hook sert exactement ça. Ne pas
//   restreindre à des rôles spécifiques sans redemander.
//
// PIN IN-GAME (confirmé en jeu 2026-07-05, tourne sans erreur) : nettoyage des marqueurs
// existants (MappinSystem) et masquage des rôles PNJ (GameplayRoleComponent) tous les deux
// fonctionnels. Note : le NPC reste fonctionnellement un policier/PNJ normal (IA/faction
// intactes) — seul le marqueur visuel disparaît, pas de conversion de type de PNJ.

public func Tessera_ApplyMapMarkers(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    FTLog(s"[Tessera/Desossage] marqueurs carte → normaux");
    return;
  }
  // Nettoyage ponctuel des marqueurs déjà enregistrés au moment de l'appel — PAS persistant (le
  // hook RegisterMappin qui aurait bloqué les nouveaux a été retiré, cf. note plus haut) : les
  // marqueurs futurs (prochaine quête, nouveau vendeur découvert) réapparaîtront tant que ce
  // levier n'est pas ré-appliqué. Suffisant pour "nettoyer maintenant", pas pour "jamais de
  // marqueur".
  let sys = GameInstance.GetMappinSystem(game);
  Tessera_ClearMappinLayer(sys, gamemappinsMappinTargetType.World);
  Tessera_ClearMappinLayer(sys, gamemappinsMappinTargetType.Minimap);
  Tessera_ClearMappinLayer(sys, gamemappinsMappinTargetType.Map);
  FTLog(s"[Tessera/Desossage] marqueurs carte → nettoyés (world+minimap+map, ponctuel)");
}

func Tessera_ClearMappinLayer(sys: ref<MappinSystem>, layer: gamemappinsMappinTargetType) -> Void {
  let entries: array<MappinEntry>;
  sys.GetMappinEntries(layer, entries);
  let i = 0;
  while i < ArraySize(entries) {
    sys.UnregisterMappin(entries[i].id);
    i += 1;
  }
}

// PNJ vendeurs/donneurs de mission : masque le marqueur de rôle au rattachement du composant.
// N'empêche PAS l'interaction/le dialogue (ça reste un vrai stub, cf. findings.md "vendors") —
// couvre uniquement l'icône visuelle au-dessus de la tête.
// CORRIGÉ (2026-07-05) : lisait `DesossageConfig.Default()` direct — bug confirmé en jeu, cf.
// `DesossageSystem.GetLiveConfig`. Utilise maintenant l'état réellement vivant du panneau.
@wrapMethod(GameplayRoleComponent)
protected func OnGameAttach() -> Void {
  wrappedMethod();
  if !DesossageSystem.GetLiveConfig(GetGameInstance()).vendors.active {
    this.SetForceHidden(true);
    this.HideRoleMappins();
  }
}

// Icône de vigilance/détection (l'œil/jauge au-dessus des PNJ pendant la furtivité) — recherche
// déléguée (Fable 5) 2026-07-05, suite au résiduel constaté après le test `gangHostility` (les
// gangs restent neutres mais l'icône de détection restait visible). Système DISTINCT de
// GameplayRoleComponent ci-dessus : c'est le "stealth mappin" (RTTI `gamemappinsStealthMappin` /
// `gameuiStealthMappinController`, alias redscript court `StealthMappinController`, même piège de
// nommage que `MappinSystem`).
// Confirmé par le script décompilé officiel (CDPR-Modding-Documentation/Cyberpunk-Scripts,
// scripts/cyberpunk/UI/mappins/stealthMappins.script) : `ShouldDisableMappin()` est le point
// d'accroche vanilla existant — quand il renvoie `true`, `OnUpdate()` masque intégralement le
// mappin (widget ET représentation 3D). Le vanilla l'utilise déjà pour les PNJ amicaux/morts ; on
// étend ce chemin à tous les cas. Signature confirmée par 2 mods publiés qui compilent
// (djkovrik/CP77Mods "Limited HUD", worldMarkersEnemy.reds ; mod Nexus "Drone Companions" #4520).
// Purement cosmétique : ne touche PAS à la perception IA (senses/stims restent intacts, les PNJ
// perçoivent toujours le joueur normalement) — seul l'affichage disparaît. Rattaché au levier
// `mapMarkers` existant plutôt qu'un nouveau, même famille "cacher les marqueurs".
// PIN IN-GAME : jamais testé — à confirmer (icône de détection absente au-dessus des PNJ).
// Classe = `gameuiStealthMappinController` (préfixe `gameui`), PAS `StealthMappinController` :
// ce dernier n'existe pas au RTTI (`StealthMappinGameController`, lui, existe mais n'expose que
// OnInitialize — c'est le game controller du widget, pas le controller du mappin). Vérifié au dump
// 2026-07-15 : ShouldDisableMappin() -> Bool n'est déclaré que sur gameuiStealthMappinController.
@wrapMethod(gameuiStealthMappinController)
private final func ShouldDisableMappin() -> Bool {
  if !DesossageSystem.GetLiveConfig(GetGameInstance()).mapMarkers.active {
    return true;
  }
  return wrappedMethod();
}
