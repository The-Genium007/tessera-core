module Tessera.UiKit

import Codeware.UI.*

// ─────────────────────────────────────────────────────────────────────────────
// LOBBY D'ARRIVÉE — v1 ink (C3/C4), premier écran du parcours joueur (implémentation
// chronologique décidée le 2026-07-23). Design validé : maquette-lobby-arrivee.html (v2, DA
// officielle DIRECTION-ARTISTIQUE.md) + spec flux d'arrivée 2026-07-18.
//
// PÉRIMÈTRE v1 (dev) : l'ÉCRAN seul — cartes de personnages (données locales de démo, la
// tranche A serveur CharacterStore viendra remplacer), sélection au clic (cadre cyan), bouton
// CONNEXION activé par la sélection, carte « Créer » (stub — sonde 1a du creator à venir).
// HORS périmètre v1 : gel GameplayTier/bulle, messages serveur, creator natif.
//
// Véhicule : InGamePopup (Codeware) comme le panneau H2 VALIDÉ EN JEU — vignette ROUGE native
// (exactement la DA du lobby), curseur, input modal, ESC pour fermer (comportement dev ; en
// prod le lobby ne sera pas fermable sans choisir). Toutes les signatures ci-dessous sont
// celles du Codeware.UI.reds local qui compile — zéro devinette.
// ─────────────────────────────────────────────────────────────────────────────
public class TesseraLobbyPopup extends InGamePopup {

  // Cadres (cell_fg) des cartes — re-teintés à la sélection (rouge = structure, cyan = sélection).
  protected let m_frames: array<wref<inkImage>>;
  protected let m_cardNames: array<String>;
  protected let m_selected: Int32;
  protected let m_connect: ref<SimpleButton>;
  protected let m_status: wref<inkText>;

  public func UseCursor() -> Bool {
    return true;
  }

  protected cb func OnCreate() {
    super.OnCreate(); // vignette rouge + m_container centré (~1550x840)
    this.m_selected = -1;

    // SENSATION DE MENU (retour Lucas 2026-07-23) : sans ceci, le lobby flottait au-dessus du
    // gameplay. Voile quasi opaque plein écran INSÉRÉ DERRIÈRE (index 0) la vignette/le contenu —
    // le monde disparaît, comme dans les menus natifs. Le flou + la dilatation du temps
    // d'InGamePopup s'ajoutent par-dessus.
    let root: wref<inkCompoundWidget> = this.GetRootCompoundWidget();
    let backdrop: ref<inkRectangle> = new inkRectangle();
    backdrop.SetName(n"backdrop");
    backdrop.SetAnchor(inkEAnchor.Fill);
    backdrop.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    backdrop.BindProperty(n"tintColor", n"MainColors.Fullscreen_PrimaryBackgroundDarkest");
    backdrop.SetOpacity(0.96);
    backdrop.Reparent(root, 0);

    // FOND GRAPHIQUE DE MENU (retour Lucas 2026-07-23 : « le fond comme derrière l'inventaire »).
    // On RÉFÉRENCE (jamais copier) les décos natives de l'atlas de formes du jeu — les mêmes
    // « fluff » que les écrans plein écran — superposées faiblement, teintées charte, DERRIÈRE la
    // vignette rouge. Reconstruction (notre composition), pas d'asset embarqué.
    let sheen: ref<inkImage> = new inkImage();
    sheen.SetName(n"bg_sheen");
    sheen.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    sheen.SetTexturePart(n"frame_gradient1");
    sheen.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    sheen.BindProperty(n"tintColor", n"MainColors.PanelRed");
    sheen.SetOpacity(0.35);
    sheen.SetAnchor(inkEAnchor.Fill);
    sheen.Reparent(root, 1);

    let fluffA: ref<inkImage> = new inkImage();
    fluffA.SetName(n"bg_fluffA");
    fluffA.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    fluffA.SetTexturePart(n"fluff_protocol1");
    fluffA.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    fluffA.BindProperty(n"tintColor", n"MainColors.Red");
    fluffA.SetOpacity(0.10);
    fluffA.SetSize(900.0, 900.0);
    fluffA.SetAnchor(inkEAnchor.TopRight);
    fluffA.SetAnchorPoint(Vector2(1.0, 0.0));
    fluffA.Reparent(root, 1);

    let fluffB: ref<inkImage> = new inkImage();
    fluffB.SetName(n"bg_fluffB");
    fluffB.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    fluffB.SetTexturePart(n"fluffcc35_3");
    fluffB.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    fluffB.BindProperty(n"tintColor", n"MainColors.Blue");
    fluffB.SetOpacity(0.07);
    fluffB.SetSize(760.0, 760.0);
    fluffB.SetAnchor(inkEAnchor.BottomLeft);
    fluffB.SetAnchorPoint(Vector2(0.0, 1.0));
    fluffB.Reparent(root, 1);

    let layout: ref<inkVerticalPanel> = new inkVerticalPanel();
    layout.SetName(n"lobby_layout");
    layout.SetAnchor(inkEAnchor.Fill);
    layout.SetMargin(inkMargin(90.0, 55.0, 90.0, 55.0));
    layout.SetChildMargin(inkMargin(0.0, 8.0, 0.0, 8.0));
    layout.Reparent(this.m_container);

    // Titre — « TESSERA // SERVEUR RP » (raj, gras, bleu de la charte).
    let title: ref<inkText> = new inkText();
    title.SetName(n"title");
    title.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    title.SetFontStyle(n"Bold");
    title.SetFontSize(54);
    title.SetLetterCase(textLetterCase.UpperCase);
    title.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    title.BindProperty(n"tintColor", n"MainColors.Blue");
    title.SetText("Tessera // Serveur RP");
    title.Reparent(layout);

    // Sous-titre.
    let sub: ref<inkText> = new inkText();
    sub.SetName(n"subtitle");
    sub.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    sub.SetFontStyle(n"Medium");
    sub.SetFontSize(30);
    sub.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    sub.BindProperty(n"tintColor", n"MainColors.ReadableMedium");
    sub.SetText("Choisis ton personnage — ou crées-en un.");
    sub.Reparent(layout);

    // Rangée de cartes.
    let row: ref<inkHorizontalPanel> = new inkHorizontalPanel();
    row.SetName(n"cards_row");
    row.SetHAlign(inkEHorizontalAlign.Center);
    row.SetChildMargin(inkMargin(0.0, 0.0, 26.0, 0.0));
    row.SetMargin(inkMargin(0.0, 20.0, 0.0, 20.0));
    row.Reparent(layout);

    // v1 : données locales de démo — remplacées plus tard par CharacterList (tranche A serveur).
    this.CreateCharacterCard(row, 0, "Vika Moreno", "Nomade · Médic", "Vu il y a 2 j · Watson");
    this.CreateCharacterCard(row, 1, "Dex Carter", "Corpo · NetWatch", "Vu il y a 9 j · City Center");
    this.CreateCreateCard(row);

    // Ligne d'état (hint), réécrite au fil des clics.
    let status: ref<inkText> = new inkText();
    status.SetName(n"status");
    status.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    status.SetFontStyle(n"Medium");
    status.SetFontSize(26);
    status.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    status.BindProperty(n"tintColor", n"MainColors.MildRed");
    status.SetText("Sélectionne un personnage pour activer la connexion.");
    status.Reparent(layout);
    this.m_status = status;

    // CONNEXION — SimpleButton (validé H2), grisé tant qu'aucun perso n'est choisi.
    this.m_connect = SimpleButton.Create();
    this.m_connect.SetName(n"btn_connect");
    this.m_connect.SetText("CONNEXION");
    this.m_connect.ToggleAnimations(true);
    this.m_connect.ToggleSounds(true);
    this.m_connect.SetDisabled(true);
    this.m_connect.Reparent(layout);
    this.m_connect.RegisterToCallback(n"OnBtnClick", this, n"OnConnect");
  }

  // Carte de personnage : canvas + cell_bg (fond nine-slice) + cell_fg (cadre, re-teinté à la
  // sélection) + pile de textes en bas. Même vocabulaire d'atlas que SimpleButton (H2).
  private func CreateCharacterCard(parent: wref<inkCompoundWidget>, index: Int32, name: String, role: String, lastSeen: String) {
    let card: ref<inkCanvas> = new inkCanvas();
    card.SetName(StringToName(s"card_\(index)"));
    card.SetSize(300.0, 420.0);
    card.SetInteractive(true);

    let bg: ref<inkImage> = new inkImage();
    bg.SetName(n"bg");
    bg.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    bg.SetTexturePart(n"cell_bg");
    bg.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    bg.BindProperty(n"tintColor", n"MainColors.Fullscreen_PrimaryBackgroundDarkest");
    bg.SetOpacity(0.85);
    bg.SetAnchor(inkEAnchor.Fill);
    bg.SetNineSliceScale(true);
    bg.SetNineSliceGrid(inkMargin(0.0, 0.0, 10.0, 0.0));
    bg.Reparent(card);

    let frame: ref<inkImage> = new inkImage();
    frame.SetName(n"frame");
    frame.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    frame.SetTexturePart(n"cell_fg");
    frame.SetTintColor(ThemeColors.RedOxide()); // repos : rouge structure (DA)
    frame.SetAnchor(inkEAnchor.Fill);
    frame.SetNineSliceScale(true);
    frame.SetNineSliceGrid(inkMargin(0.0, 0.0, 10.0, 0.0));
    frame.Reparent(card);
    ArrayPush(this.m_frames, frame);
    ArrayPush(this.m_cardNames, name);

    // Pile de textes ancrée en bas de la carte.
    let meta: ref<inkVerticalPanel> = new inkVerticalPanel();
    meta.SetName(n"meta");
    meta.SetAnchor(inkEAnchor.BottomFillHorizontaly);
    meta.SetMargin(inkMargin(18.0, 0.0, 18.0, 20.0));
    meta.SetChildMargin(inkMargin(0.0, 2.0, 0.0, 2.0));
    meta.SetFitToContent(true);
    meta.Reparent(card);

    let nameText: ref<inkText> = new inkText();
    nameText.SetName(n"name");
    nameText.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    nameText.SetFontStyle(n"Bold");
    nameText.SetFontSize(34);
    nameText.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    nameText.BindProperty(n"tintColor", n"MainColors.White");
    nameText.SetText(name);
    nameText.Reparent(meta);

    let roleText: ref<inkText> = new inkText();
    roleText.SetName(n"role");
    roleText.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    roleText.SetFontStyle(n"Medium");
    roleText.SetFontSize(22);
    roleText.SetLetterCase(textLetterCase.UpperCase);
    roleText.SetTintColor(ThemeColors.Bittersweet());
    roleText.SetText(role);
    roleText.Reparent(meta);

    let seenText: ref<inkText> = new inkText();
    seenText.SetName(n"seen");
    seenText.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    seenText.SetFontStyle(n"Medium");
    seenText.SetFontSize(20);
    seenText.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    seenText.BindProperty(n"tintColor", n"MainColors.ReadableMedium");
    seenText.SetText(lastSeen);
    seenText.Reparent(meta);

    // Un handler par carte (désambiguïsation sûre sans API de ciblage non vérifiée).
    if index == 0 {
      card.RegisterToCallback(n"OnRelease", this, n"OnCard0Release");
    } else {
      card.RegisterToCallback(n"OnRelease", this, n"OnCard1Release");
    }
    card.Reparent(parent);
  }

  // Carte « Créer un personnage » — stub v1 (le creator natif patché arrive avec la sonde 1a).
  private func CreateCreateCard(parent: wref<inkCompoundWidget>) {
    let card: ref<inkCanvas> = new inkCanvas();
    card.SetName(n"card_create");
    card.SetSize(300.0, 420.0);
    card.SetInteractive(true);

    let bg: ref<inkImage> = new inkImage();
    bg.SetName(n"bg");
    bg.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    bg.SetTexturePart(n"cell_bg");
    bg.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    bg.BindProperty(n"tintColor", n"MainColors.PanelRed");
    bg.SetOpacity(0.5);
    bg.SetAnchor(inkEAnchor.Fill);
    bg.SetNineSliceScale(true);
    bg.SetNineSliceGrid(inkMargin(0.0, 0.0, 10.0, 0.0));
    bg.Reparent(card);

    let frame: ref<inkImage> = new inkImage();
    frame.SetName(n"frame");
    frame.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    frame.SetTexturePart(n"cell_fg");
    frame.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    frame.BindProperty(n"tintColor", n"MainColors.Red");
    frame.SetAnchor(inkEAnchor.Fill);
    frame.SetNineSliceScale(true);
    frame.SetNineSliceGrid(inkMargin(0.0, 0.0, 10.0, 0.0));
    frame.Reparent(card);

    let plus: ref<inkText> = new inkText();
    plus.SetName(n"plus");
    plus.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    plus.SetFontStyle(n"Bold");
    plus.SetFontSize(110);
    plus.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    plus.BindProperty(n"tintColor", n"MainColors.Red");
    plus.SetAnchor(inkEAnchor.Centered);
    plus.SetAnchorPoint(Vector2(0.5, 0.5));
    plus.SetText("+");
    plus.Reparent(card);

    let meta: ref<inkVerticalPanel> = new inkVerticalPanel();
    meta.SetName(n"meta");
    meta.SetAnchor(inkEAnchor.BottomFillHorizontaly);
    meta.SetMargin(inkMargin(18.0, 0.0, 18.0, 20.0));
    meta.SetChildMargin(inkMargin(0.0, 2.0, 0.0, 2.0));
    meta.SetFitToContent(true);
    meta.Reparent(card);

    let label: ref<inkText> = new inkText();
    label.SetName(n"label");
    label.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    label.SetFontStyle(n"Bold");
    label.SetFontSize(28);
    label.SetTintColor(ThemeColors.Bittersweet());
    label.SetText("Créer un personnage");
    label.Reparent(meta);

    let hint: ref<inkText> = new inkText();
    hint.SetName(n"hint");
    hint.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    hint.SetFontStyle(n"Medium");
    hint.SetFontSize(20);
    hint.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    hint.BindProperty(n"tintColor", n"MainColors.ReadableMedium");
    hint.SetText("Creator natif · apparence + nom");
    hint.Reparent(meta);

    card.RegisterToCallback(n"OnRelease", this, n"OnCardCreateRelease");
    card.Reparent(parent);
  }

  protected cb func OnCard0Release(evt: ref<inkPointerEvent>) -> Bool {
    if evt.IsAction(n"click") {
      this.SelectCard(0);
    }
    return true;
  }

  protected cb func OnCard1Release(evt: ref<inkPointerEvent>) -> Bool {
    if evt.IsAction(n"click") {
      this.SelectCard(1);
    }
    return true;
  }

  protected cb func OnCardCreateRelease(evt: ref<inkPointerEvent>) -> Bool {
    if evt.IsAction(n"click") && IsDefined(this.m_status) {
      this.m_status.SetText("Création : creator natif (apparence + nom) — à venir (sonde 1a).");
    }
    return true;
  }

  private func SelectCard(index: Int32) {
    this.m_selected = index;
    let i: Int32 = 0;
    while i < ArraySize(this.m_frames) {
      if IsDefined(this.m_frames[i]) {
        if i == index {
          this.m_frames[i].SetTintColor(ThemeColors.ElectricBlue()); // sélection : cyan (DA)
        } else {
          this.m_frames[i].SetTintColor(ThemeColors.RedOxide());
        }
      }
      i += 1;
    }
    if IsDefined(this.m_status) {
      this.m_status.SetText(s"Prêt — personnage : \(this.m_cardNames[index]).");
    }
    this.m_connect.SetDisabled(false);
  }

  protected cb func OnConnect(widget: wref<inkWidget>) -> Bool {
    if this.m_selected >= 0 {
      FTLog(s"[Tessera/UiKit] lobby : CONNEXION avec \(this.m_cardNames[this.m_selected]) (v1 dev — SelectCharacter viendra ici)");
      if IsDefined(this.m_status) {
        this.m_status.SetText("Connexion au serveur… (v1 dev : fermeture du lobby)");
      }
      this.Close();
    }
    return true;
  }

  public static func Show(requester: wref<inkGameController>) {
    let popup: ref<TesseraLobbyPopup> = new TesseraLobbyPopup();
    popup.Open(requester);
  }
}
