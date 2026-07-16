module Tessera.Desossage

// ─────────────────────────────────────────────────────────────────────────────
// Gate — canal facts (2026-07-16) : journalise CHAQUE écriture de fact de quête (SetFact), pas
// seulement celles posées par nous (DesossageEvents.reds) — objectif : voir si le jeu lui-même
// pose/lit un fact de progression de campagne pendant la session d'observation (Gate, spec
// docs/superpowers/specs/2026-07-16-desossage-campagne-gelee-observation-design.md §2b).
//
// Wrap sur QuestsSystem.SetFact(CName, Int32) — signature et visibilité `public` confirmées par
// l'appel externe existant `GameInstance.GetQuestsSystem(game).SetFact(n"disable_tutorials",
// value)` dans DesossageEvents.reds (un appel externe à une méthode non publique ne compilerait
// pas). Strictement log-only : `wrappedMethod` est toujours appelée, aucun blocage.
//
// PIN IN-GAME : jamais testé — à confirmer que le log apparaît bien à chaque
// disable_tutorials/air_traffic_off posé par nous ET, si le jeu en pose un pendant le Gate, à
// chaque fact de progression de campagne.
// ─────────────────────────────────────────────────────────────────────────────
@wrapMethod(QuestsSystem)
public func SetFact(factName: CName, value: Int32) -> Void {
  FTLog(s"[Tessera/Gate/Fact] SetFact \(factName) = \(value)");
  wrappedMethod(factName, value);
}
