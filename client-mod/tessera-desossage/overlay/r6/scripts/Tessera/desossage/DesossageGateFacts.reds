module Tessera.Desossage

// ─────────────────────────────────────────────────────────────────────────────
// Gate — canal Fact NEUTRALISÉ (2026-07-16).
//
// L'implémentation d'origine (wrapMethod sur QuestsSystem.SetFact(CName, Int32)) NE COMPILE PAS :
// SetFact est une méthode NATIVE du moteur, et wrapMethod n'injecte que dans du bytecode scripté.
// Une native publique est appelable mais PAS wrappable — même classe d'échec que RegisterMappin
// (natif, même message [UNRESOLVED_METHOD], retiré le 05/07, cf. DesossageMappins.reds). Le
// raisonnement d'origine (« publique donc wrappable ») était le piège : public ≠ scriptée.
//
// Cette version cassée a été publiée dans le modset 0.1.4 (canal playtest) et rendait le jeu
// INJOUABLE : redscript refuse tout r6/scripts dès qu'un fichier échoue, faisant tomber le
// désossage entier + Cyberverse. Kick systématique / échec de compilation au lancement.
//
// La capture GLOBALE des facts posés par le jeu lui-même doit passer par un hook C++ RED4ext dans
// le fork Cyberverse (décidé 2026-07-16), pas par redscript. Nos PROPRES écritures de facts
// (disable_tutorials / air_traffic_off) sont déjà journalisées dans DesossageEvents.reds.
//
// Fichier laissé volontairement vide (compile no-op) pour ne pas faire tomber tout r6/scripts.
// ─────────────────────────────────────────────────────────────────────────────
