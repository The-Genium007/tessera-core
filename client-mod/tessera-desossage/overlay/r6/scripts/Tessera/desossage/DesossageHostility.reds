module Tessera.Desossage

// Hostilité des gangs territoriaux — recherche déléguée (Fable 5, 2026-07-05), croisée RTTI local
// + script décompilé officiel (CDPR-Modding-Documentation/Cyberpunk-Scripts, attitudeSystem.script)
// + mods publiés réels qui compilent (MaximiliumM/appearancemenumod appelle exactement
// `Game.GetAttitudeSystem():SetAttitudeRelationFromTweak(...)`).
//
// Confirmé en jeu 2026-07-05 : couper `pedestrians` (ChangeDensityModifier) n'a AUCUN effet sur
// les gangs — donc PAS le même mécanisme que piétons/trafic (contrairement à ce qu'on aurait pu
// supposer par analogie). Root cause (script décompilé) : l'hostilité des gangs n'est pas une
// question de spawn/densité mais de RELATION D'ATTITUDE entre le groupe du gang et celui du
// joueur, gérée par `AttitudeSystem` (RTTI `gameCAttitudeManager` — nom court côté redscript,
// même piège de nommage que `MappinSystem`, cf. `DesossageMappins.reds`).
//
// Piste retenue : `AttitudeSystem.SetAttitudeRelationFromTweak(groupA, groupB, attitude)` —
// PAS la variante `...Persistent` (qui écrit dans la sauvegarde ; on préfère ré-appliquer à chaque
// chargement via DesossageSystem, cohérent avec le reste du mod).
// Piste alternative NON retenue pour l'instant : wrapper `AIActionHelper
// .TryChangingAttitudeToHostile` (entonnoir unique par lequel passent tous les déclencheurs
// d'hostilité vanilla, confirmé par script décompilé + 2 mods publiés qui le @replaceMethod) —
// plus radical (bloquerait aussi une éventuelle hostilité future non couverte par les groupes
// listés ci-dessous) mais nécessite `@replaceMethod` (pas `@wrapMethod`, la fonction est `final
// static`) donc pas de `wrappedMethod()` pour préserver le comportement vanilla quand le levier est
// actif — à reconsidérer si les relations de groupe s'avèrent insuffisantes en jeu.
// PIN IN-GAME : jamais testé — à confirmer (aller en territoire de gang, vérifier qu'ils
// n'attaquent plus à vue).
public func Tessera_ApplyGangHostility(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  let sys = GameInstance.GetAttitudeSystem(game);
  let newAttitude: EAIAttitude;
  if e.active {
    newAttitude = EAIAttitude.AIA_Hostile;
  } else {
    newAttitude = EAIAttitude.AIA_Neutral;
  }
  let playerGroup = t"Attitudes.Group_Player";

  // Liste de groupes extraite d'un mod publié réel (rfuzzo/cyberpunk-nexus-script-dump, mods/19747
  // "They Will Remember", Factions.reds) — chaque gang a une variante "_OW" (open world) à traiter
  // aussi. NCPD volontairement exclu : c'est la police, déjà géré par le levier `police`
  // (PreventionSystem.OnAttach, DesossageOrder.reds) — mélanger les deux périmètres prêterait à
  // confusion.
  let groups: array<TweakDBID>;
  groups.PushBack(t"Attitudes.Group_Maelstrom");
  groups.PushBack(t"Attitudes.Group_Maelstrom_OW");
  groups.PushBack(t"Attitudes.Group_TygerClaws");
  groups.PushBack(t"Attitudes.Group_TygerClaws_OW");
  groups.PushBack(t"Attitudes.Group_Animals");
  groups.PushBack(t"Attitudes.Group_Animals_OW");
  groups.PushBack(t"Attitudes.Group_Scavenger");
  groups.PushBack(t"Attitudes.Group_Scavenger_OW");
  groups.PushBack(t"Attitudes.Group_Valentinos");
  groups.PushBack(t"Attitudes.Group_Valentinos_OW");
  groups.PushBack(t"Attitudes.Group_VoodooBoys");
  groups.PushBack(t"Attitudes.Group_VoodooBoys_OW");
  groups.PushBack(t"Attitudes.Group_6thStreet");
  groups.PushBack(t"Attitudes.Group_6thStreet_OW");
  groups.PushBack(t"Attitudes.Group_Aldecaldos");
  groups.PushBack(t"Attitudes.Group_Aldecaldos_OW");

  let i = 0;
  while i < ArraySize(groups) {
    sys.SetAttitudeRelationFromTweak(groups[i], playerGroup, newAttitude);
    i += 1;
  }
  FTLog(s"[Tessera/Desossage] hostilité gangs → \(e.active)");
}
