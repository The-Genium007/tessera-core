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
  ArrayPush(groups,t"Attitudes.Group_Maelstrom");
  ArrayPush(groups,t"Attitudes.Group_Maelstrom_OW");
  ArrayPush(groups,t"Attitudes.Group_TygerClaws");
  ArrayPush(groups,t"Attitudes.Group_TygerClaws_OW");
  ArrayPush(groups,t"Attitudes.Group_Animals");
  ArrayPush(groups,t"Attitudes.Group_Animals_OW");
  ArrayPush(groups,t"Attitudes.Group_Scavenger");
  ArrayPush(groups,t"Attitudes.Group_Scavenger_OW");
  ArrayPush(groups,t"Attitudes.Group_Valentinos");
  ArrayPush(groups,t"Attitudes.Group_Valentinos_OW");
  ArrayPush(groups,t"Attitudes.Group_VoodooBoys");
  ArrayPush(groups,t"Attitudes.Group_VoodooBoys_OW");
  ArrayPush(groups,t"Attitudes.Group_6thStreet");
  ArrayPush(groups,t"Attitudes.Group_6thStreet_OW");
  ArrayPush(groups,t"Attitudes.Group_Aldecaldos");
  ArrayPush(groups,t"Attitudes.Group_Aldecaldos_OW");

  let i = 0;
  while i < ArraySize(groups) {
    sys.SetAttitudeRelationFromTweak(groups[i], playerGroup, newAttitude);
    i += 1;
  }
  FTLog(s"[Tessera/Desossage] hostilité gangs → \(e.active)");
}

// Piste 1 (activée 2026-07-05 en complément de la relation de groupe ci-dessus) : la relation de
// groupe seule s'est révélée insuffisante en jeu — confirmé par Lucas : rester dans la zone du
// gang (ou l'avoir frappé une fois) finit par le repasser hostile malgré la relation neutre
// (un autre chemin du jeu — stims/détection de menace — re-déclenche l'escalade indépendamment de
// la relation de groupe de base). `TryChangingAttitudeToHostile` est l'entonnoir UNIQUE par lequel
// passent tous ces chemins (confirmé par script décompilé + 14 sites d'appel vanilla vérifiés) :
// bloquer ce point précis devrait couvrir tous les cas, pas seulement la relation de base.
// PIN IN-GAME : @wrapMethod tenté en premier (standard, préserve le comportement vanilla via
// wrappedMethod quand le levier est actif) même si les mods publiés de référence utilisaient
// @replaceMethod pour cette fonction précise — à confirmer si `final static` bloque le wrap ; si
// ça casse au compile, cf. alternative @replaceMethod plus haut (réimplémentation complète requise,
// plus risqué, pas encore tentée).
// CORRIGÉ (2026-07-05, dans la foulée du premier test) : lisait `DesossageConfig.Default()` direct
// — bug confirmé en jeu (fonctionnait seulement après un cocher/décocher manuel, jamais à l'état
// par défaut, cf. `DesossageSystem.GetLiveConfig`). Utilise maintenant l'état réellement vivant.
@wrapMethod(AIActionHelper)
public final static func TryChangingAttitudeToHostile(owner: ref<ScriptedPuppet>, target: ref<GameObject>) -> Bool {
  if !DesossageSystem.GetLiveConfig(GetGameInstance()).gangHostility.active && target.IsPlayer() {
    return false;
  }
  return wrappedMethod(owner, target);
}
