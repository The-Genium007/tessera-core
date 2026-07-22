module Tessera.UiKit

import Codeware.UI.*

// ─────────────────────────────────────────────────────────────────────────────
// Palier H2 — PREMIER panneau reconstruit, visible, stylé Cyberpunk.
// Objectif : prouver le pipeline de reconstruction `ink` de bout en bout (spec
// docs/superpowers/specs/2026-07-18-uikit-reconstruction-h2-design.md). Ce n'est PAS un écran
// métier — c'est un "kitchen sink" de démonstration (titre + textes stylés + rangée de boutons
// cliquables + ligne d'état) pour vérifier que : la touche ouvre un panneau, le panneau s'affiche
// avec le look natif, le curseur apparaît, et les clics arrivent.
//
// STRATÉGIE (D-U12 #2 reconstruction) : on sous-classe `InGamePopup` de Codeware plutôt que de
// bricoler l'accroche écran contre `inkSystem` à la main. `InGamePopup` (via CustomPopup +
// CustomPopupManager) gère GRATUITEMENT et de façon éprouvée : l'attache à l'écran (reparent dans
// le NotificationsContainer natif), la vignette + le cadre centré, le curseur, le focus, l'input
// modal, l'ESC-pour-fermer et le flou d'arrière-plan. C'est exactement ce que fait le mod démo
// officiel de Codeware, InkPlayground (github.com/psiberx/cp2077-playground) — notre référence qui
// COMPILE. Toutes les signatures ci-dessous sont recoupées sur le Codeware.UI.reds déployé
// (red4ext/plugins/Codeware/Scripts/) — source de vérité locale, pas de devinette.
//
// RÉSERVE (documentée) : sliders/toggles NATIFS fonctionnels exigent un `.inkwidget` (library
// resource), pas un `new inkSliderController()` — pas dans cette v1. On montre ici le vocabulaire
// sûr : canvas/panels, inkText (polices/couleurs/casse), et boutons SimpleButton animés+cliquables.
// ─────────────────────────────────────────────────────────────────────────────
public class TesseraUiKitDemoPopup extends InGamePopup {

  // Ligne d'état mise à jour au clic — prouve que l'input arrive jusqu'à nos widgets.
  protected let m_status: wref<inkText>;

  // Curseur souris + routage des clics vers le panneau : InGamePopup.OnShow pousse le contexte
  // UIGameContext.ModalPopup dès que ceci renvoie true (mécanisme réel, cf. CustomPopup.reds).
  public func UseCursor() -> Bool {
    return true;
  }

  protected cb func OnCreate() {
    super.OnCreate(); // crée la vignette + m_container (inkCanvas centré, ~1550x840)

    // Colonne verticale qui empile titre / sous-titre / boutons / état dans le cadre natif.
    let layout: ref<inkVerticalPanel> = new inkVerticalPanel();
    layout.SetName(n"tessera_demo_layout");
    layout.SetAnchor(inkEAnchor.Fill);
    layout.SetMargin(inkMargin(90.0, 80.0, 90.0, 80.0));
    layout.SetChildMargin(inkMargin(0.0, 14.0, 0.0, 14.0));
    layout.SetHAlign(inkEHorizontalAlign.Left);
    layout.Reparent(this.m_container);

    // Titre — police Rajdhani (raj) en gras, casse haute, bleu électrique (look UI native).
    let title: ref<inkText> = new inkText();
    title.SetName(n"title");
    title.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    title.SetFontStyle(n"Bold");
    title.SetFontSize(60);
    title.SetLetterCase(textLetterCase.UpperCase);
    title.SetTintColor(ThemeColors.ElectricBlue());
    title.SetText("Tessera — UIKit H2");
    title.Reparent(layout);

    // Sous-titre explicatif.
    let subtitle: ref<inkText> = new inkText();
    subtitle.SetName(n"subtitle");
    subtitle.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    subtitle.SetFontStyle(n"Medium");
    subtitle.SetFontSize(34);
    subtitle.SetTintColor(ThemeColors.Bittersweet());
    subtitle.SetText("Panneau reconstruit — preuve du pipeline. Clique un bouton. ESC pour fermer.");
    subtitle.Reparent(layout);

    // Rangée de boutons Codeware (game-styled, animés, son au clic).
    let row: ref<inkHorizontalPanel> = new inkHorizontalPanel();
    row.SetName(n"buttons_row");
    row.SetChildMargin(inkMargin(0.0, 0.0, 30.0, 0.0));
    row.SetMargin(inkMargin(0.0, 24.0, 0.0, 24.0));
    row.Reparent(layout);

    this.AddDemoButton(row, n"btn_alpha", "ALPHA");
    this.AddDemoButton(row, n"btn_beta", "BETA");
    this.AddDemoButton(row, n"btn_gamma", "GAMMA");

    // Ligne d'état, réécrite à chaque clic.
    let status: ref<inkText> = new inkText();
    status.SetName(n"status");
    status.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    status.SetFontStyle(n"Regular");
    status.SetFontSize(32);
    status.SetTintColor(ThemeColors.ElectricBlue());
    status.SetText("Aucun bouton cliqué pour l'instant.");
    status.Reparent(layout);
    this.m_status = status;
  }

  // Fabrique un SimpleButton, l'attache à `parent`, et branche son clic sur OnDemoButton.
  private func AddDemoButton(parent: wref<inkCompoundWidget>, name: CName, label: String) {
    let button: ref<SimpleButton> = SimpleButton.Create();
    button.SetName(name);
    button.SetText(label);
    button.ToggleAnimations(true);
    button.ToggleSounds(true);
    button.Reparent(parent);
    button.RegisterToCallback(n"OnBtnClick", this, n"OnDemoButton");
  }

  // Clic reçu : le widget passé est la racine du bouton cliqué (CustomButton.CallCustomCallback).
  protected cb func OnDemoButton(widget: wref<inkWidget>) -> Bool {
    if IsDefined(this.m_status) && IsDefined(widget) {
      this.m_status.SetText(s"Dernier clic : \(NameToString(widget.GetName()))");
    }
    return true;
  }

  // Ouvre le panneau. `requester` = un inkGameController vivant (cf. UiKitDemoBridge.reds).
  public static func Show(requester: wref<inkGameController>) {
    let popup: ref<TesseraUiKitDemoPopup> = new TesseraUiKitDemoPopup();
    popup.Open(requester);
  }
}
