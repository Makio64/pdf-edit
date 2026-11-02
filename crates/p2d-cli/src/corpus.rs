use std::fs;
use std::path::Path;
use p2d_writers::spdf::encode_win_ansi;
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusEntry {
    pub id: String,
    pub filename: String,
    pub category: String,
    pub page_count: usize,
    pub expected_words: Vec<String>,
    pub has_table: bool,
    pub has_acroform: bool,
    pub is_tracked_heading: bool,
    pub is_skia_zwsp: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusManifest {
    pub entries: Vec<CorpusEntry>,
}

/// Generate the 20-document synthetic French + edge-case PDF corpus in `out_dir`.
pub fn generate_corpus(out_dir: &Path) -> Result<CorpusManifest, String> {
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();

    let letter_specs = [
        (
            "01_lettre_assurance",
            "digital_letter",
            "Assurance Habitation Macif",
            vec![
                "Objet : Confirmation de prise en charge — Sinistre dégât des eaux N° SIN-2026-00000",
                "Madame, Monsieur, nous accusons réception de votre déclaration circonstanciée concernant le dégât des eaux survenu le 12 octobre 2026 dans votre appartement principal situé au 8 rue des Lilas à Paris, suite à une fuite sur canalisation privative encastrée.",
                "Après examen complet des pièces transmises par votre syndic de copropriété et validation du constat amiable contradictoire, nous avons le plaisir de vous confirmer que les dommages matériels relevés dans votre séjour et votre entrée sont intégralement couverts au titre de votre garantie Multirisque Habitation.",
                "• Franchise contractuelle applicable sur l’indemnité : 150,00 €",
                "• Plafond de remboursement garanti pour les embellissements : 12 500,00 €",
                "• Avance immédiate débloquée par virement bancaire : 1 850,00 €",
                "Nous vous prions d’agréer, Madame, l’expression de nos salutations distinguées.",
            ],
        ),
        (
            "02_lettre_impots",
            "digital_letter",
            "Direction Générale des Finances Publiques",
            vec![
                "Objet : Décision de dégrèvement partiel — Taxe foncière sur les propriétés bâties 2026",
                "Madame, par réclamation contentieuse enregistrée le 3 septembre 2026 auprès du Service des Impôts des Particuliers de Paris 6e, vous avez sollicité la révision de la valeur locative cadastrale retenue pour le calcul de votre cotisation annuelle de taxe foncière.",
                "Au terme de l’instruction de votre dossier et après vérification des justificatifs de situation familiale et de ressources que vous nous avez communiqués, un dégrèvement d’office d’un montant de 486,00 € vous est accordé sur le rôle général de l’année 2026.",
                "Le remboursement correspondant sera viré directement sur votre compte bancaire connu de nos services sous un délai de huit jours ouvrés, sans aucune démarche supplémentaire de votre part.",
                "Pour toute question complémentaire relative à ce décompte, votre centre des finances publiques demeure à votre entière disposition aux horaires habituels d’ouverture.",
            ],
        ),
        (
            "03_lettre_mutuelle",
            "digital_letter",
            "Mutuelle Santé Harmonie",
            vec![
                "Objet : Décompte détaillé de remboursement de frais médicaux et hospitaliers",
                "Bénéficiaire des prestations : Madame Marie Dupont — Adhérente N° 0001234",
                "Suite à la télétransmission Noémie reçue de votre Caisse Primaire d’Assurance Maladie concernant votre consultation spécialisée de cardiologie du 04/10/2026, nous avons procédé au règlement intégral de votre part complémentaire santé.",
                "1. Part obligatoire Sécurité sociale remboursée : 35,00 €",
                "2. Part complémentaire mutuelle versée sur votre compte : 45,00 €",
                "Aucun reste à charge ne vous est demandé pour cette consultation médicale ni pour les examens associés.",
            ],
        ),
        (
            "04_lettre_retraite",
            "digital_letter",
            "Caisse Nationale d’Assurance Vieillesse",
            vec![
                "Objet : Attestation fiscale annuelle de paiement des pensions de retraite",
                "Nous certifions que Madame Marie Dupont est titulaire d’une pension de retraite de base du régime général, versée mensuellement à terme échu le neuvième jour ouvré de chaque mois sur son compte bancaire.",
                "Montant net imposable déclaré automatiquement à l’administration fiscale pour l’année écoulée : 21 480,00 € après déduction des contributions sociales obligatoires.",
                "Conservez précieusement ce justificatif officiel pour l’ensemble de vos démarches administratives et le renouvellement de vos droits annuels.",
            ],
        ),
        (
            "05_lettre_banque",
            "digital_letter",
            "Banque Postale — Service Clientèle",
            vec![
                "Objet : Confirmation d’expédition et de renouvellement de votre carte bancaire",
                "Arrivée à échéance de validité, votre nouvelle carte bancaire internationale à puce sécurisée a été expédiée ce jour à votre domicile par pli postal suivi.",
                "Votre code confidentiel secret à quatre chiffres demeure strictement identique à celui que vous utilisez actuellement pour vos retraits et paiements.",
                "En cas de perte, de vol ou d’utilisation frauduleuse constatée sur vos relevés, contactez immédiatement notre serveur vocal du centre d’opposition.",
            ],
        ),
        (
            "06_lettre_syndic",
            "digital_letter",
            "Cabinet Immobilier du Parc — Syndic de Copropriété",
            vec![
                "Objet : Convocation officielle à l’Assemblée Générale Ordinaire des copropriétaires",
                "Chère copropriétaire, nous avons l’honneur de vous inviter à participer à la réunion annuelle de votre immeuble qui se tiendra le jeudi 19 novembre 2026 à 18h00 dans la salle de réunion du cabinet.",
                "• Approbation des comptes détaillés de l’exercice clos et quitus au syndic",
                "• Vote du budget prévisionnel et travaux d’entretien de la toiture sur cour",
                "Si vous ne pouvez pas assister personnellement à la séance, merci de nous retourner le formulaire de pouvoir ci-joint dûment daté et signé.",
            ],
        ),
        (
            "07_lettre_medecin",
            "digital_letter",
            "Compte Rendu de Consultation Médicale",
            vec![
                "Objet : Bilan clinique de suivi cardiovasculaire périodique",
                "L’examen clinique réalisé ce jour au cabinet montre un état général tout à fait satisfaisant, avec une tension artérielle parfaitement stable et régulière au repos.",
                "Nous maintenons la poursuite du traitement habituel aux mêmes posologies matin et soir pendant une durée renouvelable de trois mois consécutifs.",
                "Un bilan biologique sanguin de contrôle est prescrit à jeun avant le prochain rendez-vous de suivi programmé au début du mois de janvier.",
            ],
        ),
        (
            "08_lettre_notaire",
            "digital_letter",
            "Étude Notariale Maître Laurent",
            vec![
                "Objet : Attestation notariale de propriété immobilière et d’enregistrement",
                "Je soussigné, Maître Pierre Laurent, notaire associé à Paris, certifie et atteste que l’acte authentique relatif à votre bien immobilier a été régulièrement publié et enregistré au service de la publicité foncière.",
                "Une copie authentique exécutoire accompagnée du décompte définitif de l’étude vous est adressée sous ce pli pour votre archivage personnel.",
            ],
        ),
    ];

    for (id, cat, title, lines) in letter_specs {
        let filename = format!("{id}.pdf");
        let path = out_dir.join(&filename);
        let (bytes, words) = build_letter_pdf(title, &lines);
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
        entries.push(CorpusEntry {
            id: id.to_string(),
            filename,
            category: cat.to_string(),
            page_count: 2,
            expected_words: words,
            has_table: false,
            has_acroform: false,
            is_tracked_heading: false,
            is_skia_zwsp: false,
        });
    }

    // 09-14: Invoices & Bank Statements with Lattice Ruling Tables
    let invoice_specs = [
        (
            "09_facture_edf",
            "Facture Électricité Octobre 2026",
            vec![
                ("Abonnement mensuel 6 kVA", "12,80 €", "20 %", "15,36 €"),
                ("Consommation heures pleines", "64,20 €", "20 %", "77,04 €"),
                ("Consommation heures creuses", "28,50 €", "20 %", "34,20 €"),
                ("Total TTC à régler", "105,50 €", "20 %", "126,60 €"),
            ],
        ),
        (
            "10_facture_artisan",
            "Facture Plomberie Martin — Intervention Dépannage",
            vec![
                ("Déplacement à domicile", "40,00 €", "10 %", "44,00 €"),
                ("Remplacement joint et robinet", "85,00 €", "10 %", "93,50 €"),
                ("Main d’œuvre (1h30)", "70,00 €", "10 %", "77,00 €"),
                ("Net à payer TTC", "195,00 €", "10 %", "214,50 €"),
            ],
        ),
        (
            "11_releve_compte_1",
            "Relevé de Compte Courant — Octobre 2026",
            vec![
                ("02/10 Virement Retraite CNAV", "Crédit", "+", "1 790,00 €"),
                ("05/10 Prélèvement Électricité", "Débit", "-", "126,60 €"),
                ("11/10 Carte Pharmacie Centrale", "Débit", "-", "24,30 €"),
                ("18/10 Remboursement Mutuelle", "Crédit", "+", "80,00 €"),
            ],
        ),
        (
            "12_devis_travaux",
            "Devis Peinture Salon & Entrée",
            vec![
                ("Protection mobilier et sols", "120,00 €", "10 %", "132,00 €"),
                ("Lessivage et enduit de lissage", "340,00 €", "10 %", "374,00 €"),
                ("Deux couches peinture velours", "580,00 €", "10 %", "638,00 €"),
                ("Total Devis TTC", "1 040,00 €", "10 %", "1 144,00 €"),
            ],
        ),
        (
            "13_quittance_loyer",
            "Quittance de Garage — 4e Trimestre 2026",
            vec![
                ("Loyer mensuel octobre", "95,00 €", "0 %", "95,00 €"),
                ("Charges locatives", "12,00 €", "0 %", "12,00 €"),
                ("Total acquitté", "107,00 €", "0 %", "107,00 €"),
            ],
        ),
    ];

    for (id, title, rows) in invoice_specs {
        let filename = format!("{id}.pdf");
        let path = out_dir.join(&filename);
        let (bytes, words) = build_invoice_table_pdf(title, &rows);
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
        entries.push(CorpusEntry {
            id: id.to_string(),
            filename,
            category: "invoice_table".to_string(),
            page_count: 1,
            expected_words: words,
            has_table: true,
            has_acroform: false,
            is_tracked_heading: false,
            is_skia_zwsp: false,
        });
    }

    // 14: 2-Page Continuation Bank Statement with Repeating Headers/Footers
    {
        let id = "14_releve_multipage";
        let filename = format!("{id}.pdf");
        let path = out_dir.join(&filename);
        let (bytes, words) = build_multipage_statement_pdf();
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
        entries.push(CorpusEntry {
            id: id.to_string(),
            filename,
            category: "multipage_statement".to_string(),
            page_count: 2,
            expected_words: words,
            has_table: true,
            has_acroform: false,
            is_tracked_heading: false,
            is_skia_zwsp: false,
        });
    }

    // 15-16: Tracked Heading PDFs ("F A C T U R E", "C O U R R I E R")
    for (id, tracked_word, body_line) in [
        (
            "15_facture_tracked_heading",
            "FACTURE",
            "Montant total réglé par chèque bancaire le 15 octobre 2026 : 89,00 €.",
        ),
        (
            "16_courrier_tracked_titre",
            "ATTESTATION",
            "Ce document atteste la régularité de votre situation administrative.",
        ),
    ] {
        let filename = format!("{id}.pdf");
        let path = out_dir.join(&filename);
        let (bytes, words) = build_tracked_heading_pdf(tracked_word, body_line);
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
        entries.push(CorpusEntry {
            id: id.to_string(),
            filename,
            category: "tracked_heading".to_string(),
            page_count: 1,
            expected_words: words,
            has_table: false,
            has_acroform: false,
            is_tracked_heading: true,
            is_skia_zwsp: false,
        });
    }

    // 17-18: Chrome/Skia-style word-by-word positioned text (testing 0 glued words & French 'é' joins)
    for (id, words_list) in [
        (
            "17_chrome_skia_positioned",
            vec![
                "Récapitulatif",
                "envoyé",
                "par",
                "courrier",
                "électronique",
                "confirmé",
                "le",
                "20",
                "octobre",
                "2026.",
            ],
        ),
        (
            "18_french_accents_joining",
            vec![
                "L’électricité",
                "a",
                "été",
                "intégralement",
                "réglée",
                "auprès",
                "de",
                "la",
                "société",
                "générale.",
            ],
        ),
    ] {
        let filename = format!("{id}.pdf");
        let path = out_dir.join(&filename);
        let (bytes, words) = build_skia_word_positioned_pdf(&words_list);
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
        entries.push(CorpusEntry {
            id: id.to_string(),
            filename,
            category: "chrome_skia".to_string(),
            page_count: 1,
            expected_words: words,
            has_table: false,
            has_acroform: false,
            is_tracked_heading: false,
            is_skia_zwsp: true,
        });
    }

    // 19: Fillable AcroForm PDF
    {
        let id = "19_formulaire_cerfa_acroform";
        let filename = format!("{id}.pdf");
        let path = out_dir.join(&filename);
        let (bytes, words) = build_acroform_pdf();
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
        entries.push(CorpusEntry {
            id: id.to_string(),
            filename,
            category: "acroform".to_string(),
            page_count: 1,
            expected_words: words,
            has_table: false,
            has_acroform: true,
            is_tracked_heading: false,
            is_skia_zwsp: false,
        });
    }

    // 20: Landscape Table Document
    {
        let id = "20_paysage_tableau";
        let filename = format!("{id}.pdf");
        let path = out_dir.join(&filename);
        let (bytes, words) = build_landscape_pdf();
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
        entries.push(CorpusEntry {
            id: id.to_string(),
            filename,
            category: "landscape".to_string(),
            page_count: 1,
            expected_words: words,
            has_table: false,
            has_acroform: false,
            is_tracked_heading: false,
            is_skia_zwsp: false,
        });
    }

    // 21-26: Complex & Realistic Multi-Zone / Stream-Table / 2-Column French Documents
    for (id, cat, pages, has_tbl, builder_fn) in [
        (
            "21_avis_impot_complexe",
            "complex_tax_notice",
            2usize,
            true,
            build_complex_tax_notice_pdf as fn() -> (Vec<u8>, Vec<String>),
        ),
        (
            "22_decompte_cpam_mutuelle",
            "complex_stream_table",
            1usize,
            true,
            build_complex_cpam_statement_pdf as fn() -> (Vec<u8>, Vec<String>),
        ),
        (
            "23_facture_telecom_detaillee",
            "complex_multi_table",
            1usize,
            true,
            build_complex_multi_table_telecom_pdf as fn() -> (Vec<u8>, Vec<String>),
        ),
        (
            "24_bulletin_municipal_2colonnes",
            "complex_two_column",
            2usize,
            false,
            build_complex_two_column_bulletin_pdf as fn() -> (Vec<u8>, Vec<String>),
        ),
        (
            "25_contrat_assurance_conditions",
            "complex_mixed_layout",
            1usize,
            true,
            build_complex_mixed_contract_pdf as fn() -> (Vec<u8>, Vec<String>),
        ),
        (
            "26_releve_patrimoine_3pages",
            "complex_3page_table",
            3usize,
            true,
            build_complex_3page_patrimoine_pdf as fn() -> (Vec<u8>, Vec<String>),
        ),
    ] {
        let filename = format!("{id}.pdf");
        let path = out_dir.join(&filename);
        let (bytes, words) = builder_fn();
        fs::write(&path, bytes).map_err(|e| e.to_string())?;
        entries.push(CorpusEntry {
            id: id.to_string(),
            filename,
            category: cat.to_string(),
            page_count: pages,
            expected_words: words,
            has_table: has_tbl,
            has_acroform: false,
            is_tracked_heading: false,
            is_skia_zwsp: false,
        });
    }

    let manifest = CorpusManifest { entries };
    let manifest_json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(out_dir.join("manifest.json"), manifest_json).map_err(|e| e.to_string())?;

    Ok(manifest)
}

fn tokenize_words(s: &str) -> Vec<String> {
    s.split_whitespace()
        .filter(|w| !matches!(*w, "•" | "-" | "–" | "▪" | "+" | "|" | "1." | "2." | "3."))
        .map(|w| w.to_string())
        .collect()
}

fn wrap_line_at_chars(s: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for w in s.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + w.chars().count() > max_chars {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(w);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub(crate) fn build_letter_pdf(title: &str, lines: &[&str]) -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let font_bold_id = Ref::new(4);
    let font_italic_id = Ref::new(5);
    let page1_id = Ref::new(6);
    let content1_id = Ref::new(7);
    let page2_id = Ref::new(8);
    let content2_id = Ref::new(9);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_bold_id)
        .base_font(Name(b"Helvetica-Bold"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_italic_id)
        .base_font(Name(b"Helvetica-Oblique"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    // ========================== PAGE 1 ==========================
    let mut content1 = Content::new();
    let mut expected_words = tokenize_words(title);

    // 1. Top-Left Sender Header (aligned at y = 772..742 with Recipient block on right)
    content1.save_state();
    content1.set_fill_rgb(0.10, 0.31, 0.55); // Navy accent for sender title
    content1.begin_text();
    content1.set_font(Name(b"F2"), 11.5);
    content1.next_line(56.7, 772.0);
    content1.show(Str(&encode_win_ansi(title)));
    content1.end_text();
    content1.restore_state();

    let sender_sub = [
        "Service Relation Clientèle et Gestion des Dossiers",
        "Centre Administratif — 75006 PARIS",
    ];
    for (idx, &s) in sender_sub.iter().enumerate() {
        expected_words.extend(tokenize_words(s));
        content1.begin_text();
        content1.set_font(Name(b"F1"), 10.0);
        content1.next_line(56.7, 756.0 - (idx as f32) * 14.0);
        content1.show(Str(&encode_win_ansi(s)));
        content1.end_text();
    }

    // 2. Top-Right Recipient Block (AFNOR French letter layout at x = 350 pt, y = 772..742)
    let recip_lines = [
        ("Destinataire :", false),
        ("Madame Marie DUPONT", true),
        ("8 rue des Lilas — 75006 PARIS", false),
    ];
    for (idx, &(r_txt, is_bold)) in recip_lines.iter().enumerate() {
        expected_words.extend(tokenize_words(r_txt));
        content1.begin_text();
        content1.set_font(Name(if is_bold { b"F2" } else { b"F1" }), 10.5);
        content1.next_line(350.0, 772.0 - (idx as f32) * 15.0);
        content1.show(Str(&encode_win_ansi(r_txt)));
        content1.end_text();
    }

    // 3. Right-indented City & Date line (x = 350 pt)
    let date_line = "À Paris, le 15 octobre 2026";
    expected_words.extend(tokenize_words(date_line));
    content1.begin_text();
    content1.set_font(Name(b"F1"), 10.5);
    content1.next_line(350.0, 688.0);
    content1.show(Str(&encode_win_ansi(date_line)));
    content1.end_text();

    // 4. Page 1 Body paragraphs (multi-line paragraphs with explicit paragraph breaks)
    let mut y = 642.0f32;
    for (line_idx, &line) in lines.iter().enumerate() {
        expected_words.extend(tokenize_words(line));
        let font_name = if line_idx == 0 {
            Name(b"F2")
        } else {
            Name(b"F1")
        };
        let font_sz = if line_idx == 0 { 11.5 } else { 11.0 };
        for wrapped in wrap_line_at_chars(line, 76) {
            content1.begin_text();
            content1.set_font(font_name, font_sz);
            content1.next_line(56.7, y);
            content1.show(Str(&encode_win_ansi(&wrapped)));
            content1.end_text();
            y -= 14.5;
        }
        // Explicit paragraph break (saut de paragraphe)
        y -= 13.0;
    }

    let p1_footer = "Page 1 / 2";
    expected_words.extend(tokenize_words(p1_footer));
    content1.begin_text();
    content1.set_font(Name(b"F1"), 9.5);
    content1.next_line(268.0, 42.0);
    content1.show(Str(&encode_win_ansi(p1_footer)));
    content1.end_text();

    pdf.stream(content1_id, &content1.finish());

    let mut page1 = pdf.page(page1_id);
    page1.parent(page_tree_id);
    page1.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res1 = page1.resources();
    res1.fonts()
        .pair(Name(b"F1"), font_reg_id)
        .pair(Name(b"F2"), font_bold_id)
        .pair(Name(b"F3"), font_italic_id);
    res1.finish();
    page1.contents(content1_id);
    page1.finish();

    // ========================== PAGE 2 ==========================
    let mut content2 = Content::new();

    let p2_hdr = "Suite du courrier — Dossier suivi par votre conseiller dédié";
    expected_words.extend(tokenize_words(p2_hdr));
    content2.begin_text();
    content2.set_font(Name(b"F2"), 10.0);
    content2.next_line(56.7, 785.0);
    content2.show(Str(&encode_win_ansi(p2_hdr)));
    content2.end_text();

    let p2_sec_title = "Modalités pratiques de suivi et conservation de vos pièces justificatives";
    expected_words.extend(tokenize_words(p2_sec_title));
    content2.save_state();
    content2.set_fill_rgb(0.10, 0.31, 0.55);
    content2.begin_text();
    content2.set_font(Name(b"F2"), 12.0);
    content2.next_line(56.7, 740.0);
    content2.show(Str(&encode_win_ansi(p2_sec_title)));
    content2.end_text();
    content2.restore_state();

    let p2_paragraphs = [
        (
            Name(b"F1"),
            "Nous vous invitons à vérifier l’exactitude des informations récapitulées en première page de ce courrier et à conserver un exemplaire complet de ce document dans votre dossier personnel pendant toute la durée légale de conservation.",
        ),
        (
            Name(b"F1"),
            "Vous pouvez à tout moment transmettre un document complémentaire ou suivre l’avancement de votre dossier depuis votre espace personnel sécurisé ou par courrier postal adressé à votre centre de gestion habituel.",
        ),
        (
            Name(b"F3"),
            "Mention d’information : conformément à la réglementation relative à la protection des données personnelles, vous bénéficiez d’un droit d’accès, de rectification et d’opposition auprès de notre délégué à la protection des données.",
        ),
    ];

    let mut y2 = 706.0f32;
    for &(f_name, para_txt) in &p2_paragraphs {
        expected_words.extend(tokenize_words(para_txt));
        for wrapped in wrap_line_at_chars(para_txt, 76) {
            content2.begin_text();
            content2.set_font(f_name, 10.8);
            content2.next_line(56.7, y2);
            content2.show(Str(&encode_win_ansi(&wrapped)));
            content2.end_text();
            y2 -= 14.5;
        }
        y2 -= 14.0;
    }

    // Right-indented Signature block at bottom right (x = 350 pt)
    let sig_line = "Le Responsable du Service Clients";
    expected_words.extend(tokenize_words(sig_line));
    content2.begin_text();
    content2.set_font(Name(b"F2"), 10.5);
    content2.next_line(350.0, y2 - 18.0);
    content2.show(Str(&encode_win_ansi(sig_line)));
    content2.end_text();

    let p2_footer = "Page 2 / 2";
    expected_words.extend(tokenize_words(p2_footer));
    content2.begin_text();
    content2.set_font(Name(b"F1"), 9.5);
    content2.next_line(268.0, 42.0);
    content2.show(Str(&encode_win_ansi(p2_footer)));
    content2.end_text();

    pdf.stream(content2_id, &content2.finish());

    let mut page2 = pdf.page(page2_id);
    page2.parent(page_tree_id);
    page2.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res2 = page2.resources();
    res2.fonts()
        .pair(Name(b"F1"), font_reg_id)
        .pair(Name(b"F2"), font_bold_id)
        .pair(Name(b"F3"), font_italic_id);
    res2.finish();
    page2.contents(content2_id);
    page2.finish();

    pdf.pages(page_tree_id).kids([page1_id, page2_id]).count(2);
    (pdf.finish(), expected_words)
}

fn build_invoice_table_pdf(
    title: &str,
    rows: &[(&str, &str, &str, &str)],
) -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let font_bold_id = Ref::new(4);
    let page_id = Ref::new(5);
    let content_id = Ref::new(6);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_bold_id)
        .base_font(Name(b"Helvetica-Bold"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut content = Content::new();
    let mut expected_words = tokenize_words(title);

    content.begin_text();
    content.set_font(Name(b"F2"), 15.0);
    content.next_line(56.7, 770.0);
    content.show(Str(&encode_win_ansi(title)));
    content.end_text();

    // Draw lattice table grid lines (4 columns, header + N rows)
    let x_cols = [56.7f32, 290.0, 390.0, 460.0, 538.0];
    let top_y = 710.0f32;
    let row_h = 26.0f32;
    let total_rows = rows.len() + 1;

    content.set_line_width(0.8);
    for r in 0..=total_rows {
        let y = top_y - (r as f32) * row_h;
        content.move_to(x_cols[0], y);
        content.line_to(x_cols[4], y);
        content.stroke();
    }
    for &x in &x_cols {
        content.move_to(x, top_y);
        content.line_to(x, top_y - (total_rows as f32) * row_h);
        content.stroke();
    }

    // Header row text
    let headers = ["Désignation", "Montant HT", "TVA", "Montant TTC"];
    for (c, &h) in headers.iter().enumerate() {
        expected_words.extend(tokenize_words(h));
        content.begin_text();
        content.set_font(Name(b"F2"), 10.5);
        content.next_line(x_cols[c] + 6.0, top_y - 17.0);
        content.show(Str(&encode_win_ansi(h)));
        content.end_text();
    }

    // Data rows
    for (r_idx, &(c0, c1, c2, c3)) in rows.iter().enumerate() {
        let y = top_y - ((r_idx + 1) as f32) * row_h - 17.0;
        for (c, &cell_txt) in [c0, c1, c2, c3].iter().enumerate() {
            expected_words.extend(tokenize_words(cell_txt));
            content.begin_text();
            content.set_font(Name(b"F1"), 10.5);
            content.next_line(x_cols[c] + 6.0, y);
            content.show(Str(&encode_win_ansi(cell_txt)));
            content.end_text();
        }
    }

    pdf.stream(content_id, &content.finish());

    let mut page = pdf.page(page_id);
    page.parent(page_tree_id);
    page.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res = page.resources();
    res.fonts()
        .pair(Name(b"F1"), font_reg_id)
        .pair(Name(b"F2"), font_bold_id);
    res.finish();
    page.contents(content_id);
    page.finish();

    pdf.pages(page_tree_id).kids([page_id]).count(1);
    (pdf.finish(), expected_words)
}

fn build_multipage_statement_pdf() -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let font_bold_id = Ref::new(4);
    let page1_id = Ref::new(5);
    let content1_id = Ref::new(6);
    let page2_id = Ref::new(7);
    let content2_id = Ref::new(8);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_bold_id)
        .base_font(Name(b"Helvetica-Bold"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut expected_words = Vec::new();
    let x_cols = [56.7f32, 360.0, 538.0];

    for (p_idx, (p_id, c_id, items)) in [
        (
            page1_id,
            content1_id,
            vec![
                ("01/10 Solde précédent créditeur", "2 410,00 €"),
                ("04/10 Retrait Distributeur Paris", "60,00 €"),
            ],
        ),
        (
            page2_id,
            content2_id,
            vec![
                ("15/10 Virement Pension Retraite", "1 790,00 €"),
                ("28/10 Nouveau solde créditeur", "4 140,00 €"),
            ],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut content = Content::new();
        let hdr = "Banque Postale — Relevé Mensuel";
        expected_words.extend(tokenize_words(hdr));
        content.begin_text();
        content.set_font(Name(b"F2"), 10.0);
        content.next_line(56.7, 800.0);
        content.show(Str(&encode_win_ansi(hdr)));
        content.end_text();

        let top_y = 720.0f32;
        let row_h = 26.0f32;
        let total_rows = items.len() + 1;
        content.set_line_width(0.8);
        for r in 0..=total_rows {
            let y = top_y - (r as f32) * row_h;
            content.move_to(x_cols[0], y);
            content.line_to(x_cols[2], y);
            content.stroke();
        }
        for &x in &x_cols {
            content.move_to(x, top_y);
            content.line_to(x, top_y - (total_rows as f32) * row_h);
            content.stroke();
        }

        for (c, &h) in ["Opération", "Montant"].iter().enumerate() {
            expected_words.extend(tokenize_words(h));
            content.begin_text();
            content.set_font(Name(b"F2"), 10.5);
            content.next_line(x_cols[c] + 6.0, top_y - 17.0);
            content.show(Str(&encode_win_ansi(h)));
            content.end_text();
        }

        for (r_idx, (op, amt)) in items.iter().enumerate() {
            let y = top_y - ((r_idx + 1) as f32) * row_h - 17.0;
            for (c, &txt) in [op, amt].iter().enumerate() {
                expected_words.extend(tokenize_words(txt));
                content.begin_text();
                content.set_font(Name(b"F1"), 10.5);
                content.next_line(x_cols[c] + 6.0, y);
                content.show(Str(&encode_win_ansi(txt)));
                content.end_text();
            }
        }

        let ftr = format!("Page {} / 2", p_idx + 1);
        expected_words.extend(tokenize_words(&ftr));
        content.begin_text();
        content.set_font(Name(b"F1"), 9.5);
        content.next_line(260.0, 40.0);
        content.show(Str(&encode_win_ansi(&ftr)));
        content.end_text();

        pdf.stream(c_id, &content.finish());

        let mut page = pdf.page(p_id);
        page.parent(page_tree_id);
        page.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
        let mut res = page.resources();
        res.fonts()
            .pair(Name(b"F1"), font_reg_id)
            .pair(Name(b"F2"), font_bold_id);
        res.finish();
        page.contents(c_id);
        page.finish();
    }

    pdf.pages(page_tree_id).kids([page1_id, page2_id]).count(2);
    (pdf.finish(), expected_words)
}

fn build_tracked_heading_pdf(tracked_word: &str, body_line: &str) -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let font_bold_id = Ref::new(4);
    let page_id = Ref::new(5);
    let content_id = Ref::new(6);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_bold_id)
        .base_font(Name(b"Helvetica-Bold"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut content = Content::new();
    let mut expected_words = vec![tracked_word.to_string()];
    expected_words.extend(tokenize_words(body_line));

    // Emit each letter of `tracked_word` with positive `Tc` character spacing (3.5 pt)
    // A naive extractor turns this into "F A C T U R E", whereas our Tc-aware 2-means clusterer keeps "FACTURE"!
    content.begin_text();
    content.set_font(Name(b"F2"), 18.0);
    content.set_char_spacing(4.2);
    content.next_line(56.7, 760.0);
    content.show(Str(&encode_win_ansi(tracked_word)));
    content.set_char_spacing(0.0);
    content.end_text();

    content.begin_text();
    content.set_font(Name(b"F1"), 11.5);
    content.next_line(56.7, 715.0);
    content.show(Str(&encode_win_ansi(body_line)));
    content.end_text();

    pdf.stream(content_id, &content.finish());

    let mut page = pdf.page(page_id);
    page.parent(page_tree_id);
    page.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res = page.resources();
    res.fonts()
        .pair(Name(b"F1"), font_reg_id)
        .pair(Name(b"F2"), font_bold_id);
    res.finish();
    page.contents(content_id);
    page.finish();

    pdf.pages(page_tree_id).kids([page_id]).count(1);
    (pdf.finish(), expected_words)
}

fn build_skia_word_positioned_pdf(words_list: &[&str]) -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let page_id = Ref::new(4);
    let content_id = Ref::new(5);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut content = Content::new();
    let expected_words: Vec<String> = words_list.iter().map(|s| (*s).to_string()).collect();

    // Emulate Chrome/Skia positioning: each word is drawn in its own text object with no space glyph
    let mut x = 56.7f32;
    let y = 740.0f32;
    for &w in words_list {
        content.begin_text();
        content.set_font(Name(b"F1"), 11.5);
        content.next_line(x, y);
        content.show(Str(&encode_win_ansi(w)));
        content.end_text();
        let approx_w: f32 = w
            .chars()
            .map(|c| match c {
                'i' | 'l' | '’' | '\'' | '.' | ',' => 3.2,
                'm' | 'w' | 'M' | 'W' => 9.5,
                c if c.is_uppercase() => 7.8,
                _ => 6.4,
            })
            .sum();
        x += approx_w + 4.2;
    }

    pdf.stream(content_id, &content.finish());

    let mut page = pdf.page(page_id);
    page.parent(page_tree_id);
    page.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res = page.resources();
    res.fonts().pair(Name(b"F1"), font_reg_id);
    res.finish();
    page.contents(content_id);
    page.finish();

    pdf.pages(page_tree_id).kids([page_id]).count(1);
    (pdf.finish(), expected_words)
}

fn build_acroform_pdf() -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let page_id = Ref::new(4);
    let content_id = Ref::new(5);
    let annot_id = Ref::new(6);

    let mut cat = pdf.catalog(catalog_id);
    cat.pages(page_tree_id);
    {
        let mut acro = cat.insert(Name(b"AcroForm")).dict();
        acro.insert(Name(b"Fields")).array().item(annot_id);
    }
    cat.finish();

    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut content = Content::new();
    let lines = [
        "Formulaire de Demande Administrative",
        "Nom et Prénom du demandeur : Madame Marie Dupont",
        "Adresse postale : 8 rue des Lilas, 75006 Paris",
    ];
    let mut expected_words = Vec::new();
    let mut y = 760.0f32;
    for l in lines {
        expected_words.extend(tokenize_words(l));
        content.begin_text();
        content.set_font(Name(b"F1"), 12.0);
        content.next_line(56.7, y);
        content.show(Str(&encode_win_ansi(l)));
        content.end_text();
        y -= 28.0;
    }
    pdf.stream(content_id, &content.finish());

    let mut widget = pdf. indirect(annot_id).dict();
    widget.pair(Name(b"Type"), Name(b"Annot"));
    widget.pair(Name(b"Subtype"), Name(b"Widget"));
    widget.pair(Name(b"FT"), Name(b"Tx"));
    widget.pair(Name(b"T"), Str(b"nom_demandeur"));
    widget.pair(Name(b"V"), Str(b"Marie Dupont"));
    widget.pair(Name(b"Rect"), Rect::new(220.0, 725.0, 450.0, 745.0));
    widget.finish();

    let mut page = pdf.page(page_id);
    page.parent(page_tree_id);
    page.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    page.insert(Name(b"Annots")).array().item(annot_id);
    let mut res = page.resources();
    res.fonts().pair(Name(b"F1"), font_reg_id);
    res.finish();
    page.contents(content_id);
    page.finish();

    pdf.pages(page_tree_id).kids([page_id]).count(1);
    (pdf.finish(), expected_words)
}

fn build_landscape_pdf() -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let page_id = Ref::new(4);
    let content_id = Ref::new(5);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let line = "Tableau Récapitulatif Annuel en Format Paysage — Exercice 2026";
    let expected_words = tokenize_words(line);

    let mut content = Content::new();
    content.begin_text();
    content.set_font(Name(b"F1"), 13.0);
    content.next_line(56.7, 520.0);
    content.show(Str(&encode_win_ansi(line)));
    content.end_text();
    pdf.stream(content_id, &content.finish());

    let mut page = pdf.page(page_id);
    page.parent(page_tree_id);
    page.media_box(Rect::new(0.0, 0.0, 841.89, 595.28));
    let mut res = page.resources();
    res.fonts().pair(Name(b"F1"), font_reg_id);
    res.finish();
    page.contents(content_id);
    page.finish();

    pdf.pages(page_tree_id).kids([page_id]).count(1);
    (pdf.finish(), expected_words)
}


/// 21: Realistic 2-Page French Tax Notice (`Avis d'Impôt sur le Revenu — DGFiP`):
/// - Page 1: Dark filled header banner with white bold text + side-by-side Sender/Recipient + multi-line paragraphs + 4-column Stream Table + summary callout
/// - Page 2: Dark filled section banner + multi-line paragraphs with paragraph breaks + bulleted list + italic legal notice
fn build_complex_tax_notice_pdf() -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let font_bold_id = Ref::new(4);
    let font_italic_id = Ref::new(5);
    let page1_id = Ref::new(6);
    let content1_id = Ref::new(7);
    let page2_id = Ref::new(8);
    let content2_id = Ref::new(9);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_bold_id)
        .base_font(Name(b"Helvetica-Bold"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_italic_id)
        .base_font(Name(b"Helvetica-Oblique"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut content = Content::new();
    let mut expected_words = Vec::new();

    // 1. Dark blue banner at top with white text
    content.save_state();
    content.set_fill_rgb(0.07, 0.21, 0.36);
    content.rect(45.0, 765.0, 505.0, 42.0);
    content.fill_even_odd();
    content.restore_state();

    let banner_title = "RÉPUBLIQUE FRANÇAISE — AVIS D’IMPÔT 2026 SUR LES REVENUS 2025";
    expected_words.extend(tokenize_words(banner_title));
    content.save_state();
    content.set_fill_rgb(1.0, 1.0, 1.0); // White text on dark banner!
    content.begin_text();
    content.set_font(Name(b"F2"), 13.0);
    content.next_line(56.7, 782.0);
    content.show(Str(&encode_win_ansi(banner_title)));
    content.end_text();
    content.restore_state();

    // 2. Side-by-side Sender (left) + Recipient (right) at matching Y coordinates (730..670)
    let sender_lines = [
        "Direction Générale des Finances Publiques",
        "SIP Paris 6e — 9 place Saint-Sulpice",
        "Numéro fiscal : 00 00 000 000 000",
        "Référence de l’avis : 00 00 000 00000",
    ];
    let recipient_lines = [
        "Destinataire de l’avis :",
        "Madame Marie DUPONT",
        "8 rue des Lilas",
        "75006 PARIS",
    ];
    for (idx, &s_line) in sender_lines.iter().enumerate() {
        let y = 732.0 - (idx as f32) * 15.0;
        expected_words.extend(tokenize_words(s_line));
        content.begin_text();
        content.set_font(Name(if idx == 0 { b"F2" } else { b"F1" }), 10.0);
        content.next_line(56.7, y);
        content.show(Str(&encode_win_ansi(s_line)));
        content.end_text();
    }
    for (idx, &r_line) in recipient_lines.iter().enumerate() {
        let y = 732.0 - (idx as f32) * 15.0;
        expected_words.extend(tokenize_words(r_line));
        content.begin_text();
        content.set_font(Name(if idx == 1 { b"F2" } else { b"F1" }), 10.5);
        content.next_line(340.0, y);
        content.show(Str(&encode_win_ansi(r_line)));
        content.end_text();
    }

    // 3. Two multi-line introductory paragraphs with explicit paragraph break
    let intro_paras = [
        "Décompte détaillé de votre impôt sur le revenu calculé à partir des éléments déclarés au printemps 2026 et des retenues à la source déjà prélevées par vos organismes verseurs au cours de l’année 2025.",
        "Cet avis récapitule l’ensemble des revenus imposables de votre foyer fiscal ainsi que les abattements forfaitaires légaux appliqués automatiquement par l’administration fiscale.",
    ];
    let mut y = 640.0f32;
    for intro in intro_paras {
        expected_words.extend(tokenize_words(intro));
        for wrapped in wrap_line_at_chars(intro, 78) {
            content.begin_text();
            content.set_font(Name(b"F1"), 10.8);
            content.next_line(56.7, y);
            content.show(Str(&encode_win_ansi(&wrapped)));
            content.end_text();
            y -= 14.5;
        }
        y -= 12.0;
    }

    // 4. Stream Table (4 columns aligned by X coordinates, NO vertical lines!)
    let col_x = [56.7f32, 265.0, 365.0, 465.0];
    let stream_rows = [
        ("Rubrique fiscale", "Déclarant 1", "Abattement", "Revenu net"),
        ("Pensions et retraites", "23 860,00 €", "2 386,00 €", "21 474,00 €"),
        ("Revenus des capitaux", "1 420,00 €", "0,00 €", "1 420,00 €"),
        ("CSG déductible", "1 114,00 €", "0,00 €", "-1 114,00 €"),
        ("Revenu global imposable", "25 280,00 €", "2 386,00 €", "21 780,00 €"),
    ];

    let mut table_y = 540.0f32;
    for (r_idx, &(c0, c1, c2, c3)) in stream_rows.iter().enumerate() {
        let font = if r_idx == 0 || r_idx == 4 {
            Name(b"F2")
        } else {
            Name(b"F1")
        };
        for (c_idx, &cell) in [c0, c1, c2, c3].iter().enumerate() {
            expected_words.extend(tokenize_words(cell));
            content.begin_text();
            content.set_font(font, 10.0);
            content.next_line(col_x[c_idx], table_y);
            content.show(Str(&encode_win_ansi(cell)));
            content.end_text();
        }
        table_y -= 20.0;
    }

    // 5. Bottom summary callout line (tabbed label + amount)
    let summary_left = "Solde d’impôt restant à payer avant le 16/11/2026";
    let summary_right = "342,00 €";
    expected_words.extend(tokenize_words(summary_left));
    expected_words.extend(tokenize_words(summary_right));
    content.begin_text();
    content.set_font(Name(b"F2"), 12.0);
    content.next_line(56.7, 405.0);
    content.show(Str(&encode_win_ansi(summary_left)));
    content.end_text();
    content.begin_text();
    content.set_font(Name(b"F2"), 12.0);
    content.next_line(465.0, 405.0);
    content.show(Str(&encode_win_ansi(summary_right)));
    content.end_text();

    // 6. 90-degree rotated vertical archive stamp in left margin (x = 22 pt)
    let archive_stamp = "DGFIP-ARCHIVE-2026";
    expected_words.extend(tokenize_words(archive_stamp));
    content.save_state();
    content.begin_text();
    content.set_font(Name(b"F1"), 8.5);
    content.set_text_matrix([0.0, 1.0, -1.0, 0.0, 22.0, 320.0]);
    content.show(Str(&encode_win_ansi(archive_stamp)));
    content.end_text();
    content.restore_state();

    let p1_ftr = "Page 1 / 2";
    expected_words.extend(tokenize_words(p1_ftr));
    content.begin_text();
    content.set_font(Name(b"F1"), 9.5);
    content.next_line(268.0, 42.0);
    content.show(Str(&encode_win_ansi(p1_ftr)));
    content.end_text();

    pdf.stream(content1_id, &content.finish());

    let mut page1 = pdf.page(page1_id);
    page1.parent(page_tree_id);
    page1.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res1 = page1.resources();
    res1.fonts()
        .pair(Name(b"F1"), font_reg_id)
        .pair(Name(b"F2"), font_bold_id)
        .pair(Name(b"F3"), font_italic_id);
    res1.finish();
    page1.contents(content1_id);
    page1.finish();

    // ========================== PAGE 2 ==========================
    let mut content2 = Content::new();

    content2.save_state();
    content2.set_fill_rgb(0.07, 0.21, 0.36);
    content2.rect(45.0, 765.0, 505.0, 38.0);
    content2.fill_even_odd();
    content2.restore_state();

    let p2_banner = "MODALITÉS DE RÈGLEMENT ET TAUX DE PRÉLÈVEMENT À LA SOURCE";
    expected_words.extend(tokenize_words(p2_banner));
    content2.save_state();
    content2.set_fill_rgb(1.0, 1.0, 1.0);
    content2.begin_text();
    content2.set_font(Name(b"F2"), 12.5);
    content2.next_line(56.7, 780.0);
    content2.show(Str(&encode_win_ansi(p2_banner)));
    content2.end_text();
    content2.restore_state();

    let p2_blocks = [
        (
            Name(b"F2"),
            11.5f32,
            "1. Comment régler votre solde d’impôt sur le revenu",
        ),
        (
            Name(b"F1"),
            10.8,
            "Le solde restant dû d’un montant de 342,00 € sera automatiquement prélevé sur votre compte bancaire connu de l’administration fiscale le 16 novembre 2026. Vous n’avez aucun chèque ni virement manuel à adresser à votre centre des finances publiques.",
        ),
        (
            Name(b"F1"),
            10.8,
            "• Paiement direct en ligne depuis votre espace particulier sur le site officiel impots.gouv.fr",
        ),
        (
            Name(b"F1"),
            10.8,
            "• Application mobile Impots.gouv par lecture du code sécurisé figurant au bas de votre avis",
        ),
        (
            Name(b"F2"),
            11.5,
            "2. Actualisation de votre taux de prélèvement à la source",
        ),
        (
            Name(b"F1"),
            10.8,
            "À compter du 1er septembre 2026, le taux personnalisé appliqué à vos pensions de retraite est fixé à 4,3 %. Ce taux est transmis automatiquement par la Direction Générale des Finances Publiques à vos caisses de retraite.",
        ),
        (
            Name(b"F3"),
            10.0,
            "Voies et délais de recours : si vous souhaitez contester les éléments de calcul figurant sur cet avis, vous pouvez déposer une réclamation jusqu’au 31 décembre 2028 depuis votre messagerie sécurisée.",
        ),
    ];

    let mut y2 = 725.0f32;
    for &(f_name, f_size, txt) in &p2_blocks {
        expected_words.extend(tokenize_words(txt));
        for wrapped in wrap_line_at_chars(txt, 78) {
            content2.begin_text();
            content2.set_font(f_name, f_size);
            content2.next_line(56.7, y2);
            content2.show(Str(&encode_win_ansi(&wrapped)));
            content2.end_text();
            y2 -= 14.5;
        }
        y2 -= 13.0;
    }

    let p2_ftr = "Page 2 / 2";
    expected_words.extend(tokenize_words(p2_ftr));
    content2.begin_text();
    content2.set_font(Name(b"F1"), 9.5);
    content2.next_line(268.0, 42.0);
    content2.show(Str(&encode_win_ansi(p2_ftr)));
    content2.end_text();

    pdf.stream(content2_id, &content2.finish());

    let mut page2 = pdf.page(page2_id);
    page2.parent(page_tree_id);
    page2.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res2 = page2.resources();
    res2.fonts()
        .pair(Name(b"F1"), font_reg_id)
        .pair(Name(b"F2"), font_bold_id)
        .pair(Name(b"F3"), font_italic_id);
    res2.finish();
    page2.contents(content2_id);
    page2.finish();

    pdf.pages(page_tree_id).kids([page1_id, page2_id]).count(2);
    (pdf.finish(), expected_words)
}

/// 22: Realistic CPAM + Mutuelle Medical Reimbursement Statement (5-column borderless Stream Table)
fn build_complex_cpam_statement_pdf() -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let font_bold_id = Ref::new(4);
    let page_id = Ref::new(5);
    let content_id = Ref::new(6);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_bold_id)
        .base_font(Name(b"Helvetica-Bold"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut content = Content::new();
    let mut expected_words = Vec::new();

    let title = "Assurance Maladie — Relevé Mensuel de Remboursements de Soins";
    expected_words.extend(tokenize_words(title));
    content.begin_text();
    content.set_font(Name(b"F2"), 14.0);
    content.next_line(56.7, 780.0);
    content.show(Str(&encode_win_ansi(title)));
    content.end_text();

    // Side-by-side CPAM center (left) + Insured beneficiary (right)
    let left_hdr = [
        "Caisse Primaire d’Assurance Maladie de Paris",
        "Service Prestations Santé",
        "N° Sécurité Sociale : 2 00 00 00 000 000",
    ];
    let right_hdr = [
        "Assurée bénéficiaire :",
        "Madame Marie DUPONT",
        "8 rue des Lilas, 75006 PARIS",
    ];
    for (i, &l) in left_hdr.iter().enumerate() {
        expected_words.extend(tokenize_words(l));
        content.begin_text();
        content.set_font(Name(b"F1"), 10.0);
        content.next_line(56.7, 738.0 - (i as f32) * 15.0);
        content.show(Str(&encode_win_ansi(l)));
        content.end_text();
    }
    for (i, &r) in right_hdr.iter().enumerate() {
        expected_words.extend(tokenize_words(r));
        content.begin_text();
        content.set_font(Name(if i == 1 { b"F2" } else { b"F1" }), 10.0);
        content.next_line(340.0, 738.0 - (i as f32) * 15.0);
        content.show(Str(&encode_win_ansi(r)));
        content.end_text();
    }

    // 5-column Stream Table (Date | Nature des soins | Base Sécu | Taux | Montant versé)
    let col_x = [56.7f32, 125.0, 315.0, 405.0, 468.0];
    let rows = [
        ("Date", "Nature des soins médicaux", "Base tarif", "Taux", "Montant versé"),
        ("03/10/2026", "Consultation cardiologue", "55,00 €", "70 %", "36,50 €"),
        ("03/10/2026", "Électrocardiogramme repos", "14,26 €", "70 %", "9,98 €"),
        ("08/10/2026", "Laboratoire biologie sang", "48,50 €", "60 %", "28,10 €"),
        ("09/10/2026", "Pharmacie médicaments", "67,40 €", "65 %", "43,82 €"),
    ];

    let mut y = 650.0f32;
    for (r_idx, &(c0, c1, c2, c3, c4)) in rows.iter().enumerate() {
        let f = if r_idx == 0 { Name(b"F2") } else { Name(b"F1") };
        for (c_idx, &txt) in [c0, c1, c2, c3, c4].iter().enumerate() {
            expected_words.extend(tokenize_words(txt));
            content.begin_text();
            content.set_font(f, 9.8);
            content.next_line(col_x[c_idx], y);
            content.show(Str(&encode_win_ansi(txt)));
            content.end_text();
        }
        y -= 20.0;
    }

    let footer_note = "Ce décompte a été transmis automatiquement à votre mutuelle complémentaire Harmonie par télétransmission Noémie.";
    expected_words.extend(tokenize_words(footer_note));
    let mut ny = 515.0f32;
    for wrapped in wrap_line_at_chars(footer_note, 80) {
        content.begin_text();
        content.set_font(Name(b"F1"), 10.5);
        content.next_line(56.7, ny);
        content.show(Str(&encode_win_ansi(&wrapped)));
        content.end_text();
        ny -= 15.0;
    }

    pdf.stream(content_id, &content.finish());

    let mut page = pdf.page(page_id);
    page.parent(page_tree_id);
    page.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res = page.resources();
    res.fonts()
        .pair(Name(b"F1"), font_reg_id)
        .pair(Name(b"F2"), font_bold_id);
    res.finish();
    page.contents(content_id);
    page.finish();

    pdf.pages(page_tree_id).kids([page_id]).count(1);
    (pdf.finish(), expected_words)
}

/// 23: Multi-Table Telecom Invoice (`Orange Fibre + Téléphonie Fixe`) with 2 separate Lattice Tables on the same page
fn build_complex_multi_table_telecom_pdf() -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let font_bold_id = Ref::new(4);
    let page_id = Ref::new(5);
    let content_id = Ref::new(6);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_bold_id)
        .base_font(Name(b"Helvetica-Bold"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut content = Content::new();
    let mut expected_words = Vec::new();

    let title = "Facture Mensuelle Télécom et Fibre — Octobre 2026";
    expected_words.extend(tokenize_words(title));
    content.begin_text();
    content.set_font(Name(b"F2"), 15.0);
    content.next_line(56.7, 780.0);
    content.show(Str(&encode_win_ansi(title)));
    content.end_text();

    let x_cols = [56.7f32, 310.0, 425.0, 538.0];

    let mut draw_table = |section_heading: &str, top_y: f32, rows: &[(&str, &str, &str)]| {
        expected_words.extend(tokenize_words(section_heading));
        content.begin_text();
        content.set_font(Name(b"F2"), 12.0);
        content.next_line(56.7, top_y + 14.0);
        content.show(Str(&encode_win_ansi(section_heading)));
        content.end_text();

        let row_h = 24.0f32;
        let total_rows = rows.len();
        content.set_line_width(0.75);
        for r in 0..=total_rows {
            let y = top_y - (r as f32) * row_h;
            content.move_to(x_cols[0], y);
            content.line_to(x_cols[3], y);
            content.stroke();
        }
        for &x in &x_cols {
            content.move_to(x, top_y);
            content.line_to(x, top_y - (total_rows as f32) * row_h);
            content.stroke();
        }

        for (r_idx, &(c0, c1, c2)) in rows.iter().enumerate() {
            let y = top_y - (r_idx as f32) * row_h - 16.0;
            let font = if r_idx == 0 { Name(b"F2") } else { Name(b"F1") };
            for (c_idx, &txt) in [c0, c1, c2].iter().enumerate() {
                expected_words.extend(tokenize_words(txt));
                content.begin_text();
                content.set_font(font, 10.0);
                content.next_line(x_cols[c_idx] + 6.0, y);
                content.show(Str(&encode_win_ansi(txt)));
                content.end_text();
            }
        }
    };

    draw_table(
        "Rubrique A — Vos abonnements et forfaits mensuels",
        715.0,
        &[
            ("Abonnement mensuel", "Période facturée", "Montant TTC"),
            ("Forfait Fibre Maison et Téléphone Fixe", "01/10 au 31/10", "39,99 €"),
            ("Option Appels illimités vers mobiles", "01/10 au 31/10", "5,00 €"),
        ],
    );

    draw_table(
        "Rubrique B — Vos communications et services ponctuels",
        565.0,
        &[
            ("Nature de la communication", "Durée ou volume", "Montant TTC"),
            ("Appels internationaux vers Suisse", "24 minutes", "6,40 €"),
            ("Location boîtier télévision salon", "1 mois", "3,50 €"),
        ],
    );

    let summary = "Montant total TTC prélevé automatiquement le 05/11/2026 : 54,89 €";
    expected_words.extend(tokenize_words(summary));
    content.begin_text();
    content.set_font(Name(b"F2"), 11.5);
    content.next_line(56.7, 440.0);
    content.show(Str(&encode_win_ansi(summary)));
    content.end_text();

    pdf.stream(content_id, &content.finish());

    let mut page = pdf.page(page_id);
    page.parent(page_tree_id);
    page.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res = page.resources();
    res.fonts()
        .pair(Name(b"F1"), font_reg_id)
        .pair(Name(b"F2"), font_bold_id);
    res.finish();
    page.contents(content_id);
    page.finish();

    pdf.pages(page_tree_id).kids([page_id]).count(1);
    (pdf.finish(), expected_words)
}

/// 24: Realistic 2-Page 2-Column Municipal Senior Bulletin (`Bulletin d'Information Seniors — Mairie de Paris 6e`)
/// Tests that Column 1 is read completely top-to-bottom before Column 2, with zero cross-gutter sentence mixing, followed by Page 2!
fn build_complex_two_column_bulletin_pdf() -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let font_bold_id = Ref::new(4);
    let font_italic_id = Ref::new(5);
    let page1_id = Ref::new(6);
    let content1_id = Ref::new(7);
    let page2_id = Ref::new(8);
    let content2_id = Ref::new(9);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_bold_id)
        .base_font(Name(b"Helvetica-Bold"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_italic_id)
        .base_font(Name(b"Helvetica-Oblique"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut content = Content::new();
    let mut expected_words = Vec::new();

    // Full-width top banner
    let banner = "MAIRIE DU 6e ARRONDISSEMENT — BULLETIN D’INFORMATION SENIORS";
    expected_words.extend(tokenize_words(banner));
    content.begin_text();
    content.set_font(Name(b"F2"), 14.5);
    content.next_line(56.7, 780.0);
    content.show(Str(&encode_win_ansi(banner)));
    content.end_text();

    let subtitle = "Actualités pratiques, permanences administratives et ateliers numériques du trimestre d’automne 2026.";
    expected_words.extend(tokenize_words(subtitle));
    content.begin_text();
    content.set_font(Name(b"F1"), 11.0);
    content.next_line(56.7, 755.0);
    content.show(Str(&encode_win_ansi(subtitle)));
    content.end_text();

    // Left column (x = 56.7, width ~215 pt -> ~36 chars per line, 10 lines -> forces 2-column body mode)
    let left_col_lines = [
        ("F2", "Permanences d’aide administrative"),
        ("F1", "Les conseillers municipaux vous"),
        ("F1", "accueillent chaque mardi matin"),
        ("F1", "sans rendez-vous au rez-de-chaussée"),
        ("F1", "pour vérifier vos dossiers fiscaux."),
        ("F2", "Ateliers informatiques gratuits"),
        ("F1", "Apprenez à classer vos courriers"),
        ("F1", "électroniques et modifier vos"),
        ("F1", "documents sur votre ordinateur"),
        ("F1", "portable tous les jeudis à 14h30."),
    ];

    // Right column (x = 320.0, at the exact same Y heights!)
    let right_col_lines = [
        ("F2", "Inscriptions aux sorties culturelles"),
        ("F1", "Les visites guidées du musée"),
        ("F1", "du Luxembourg reprennent dès"),
        ("F1", "le mois de novembre pour tous"),
        ("F1", "les habitants inscrits au club."),
        ("F2", "Numéros pratiques de proximité"),
        ("F1", "Le service d’écoute et d’entraide"),
        ("F1", "de votre quartier répond du"),
        ("F1", "lundi au samedi pour toute"),
        ("F1", "livraison de livres à domicile."),
    ];

    // Expected words in natural reading order: all of Left Column, then all of Right Column!
    for &(_f, txt) in &left_col_lines {
        expected_words.extend(tokenize_words(txt));
    }
    for &(_f, txt) in &right_col_lines {
        expected_words.extend(tokenize_words(txt));
    }

    for (idx, &(f_name, txt)) in left_col_lines.iter().enumerate() {
        let y = 700.0 - (idx as f32) * 18.0;
        content.begin_text();
        content.set_font(Name(f_name.as_bytes()), 10.5);
        content.next_line(56.7, y);
        content.show(Str(&encode_win_ansi(txt)));
        content.end_text();
    }

    for (idx, &(f_name, txt)) in right_col_lines.iter().enumerate() {
        let y = 700.0 - (idx as f32) * 18.0;
        content.begin_text();
        content.set_font(Name(f_name.as_bytes()), 10.5);
        content.next_line(320.0, y);
        content.show(Str(&encode_win_ansi(txt)));
        content.end_text();
    }

    // Full-width bottom notice
    let bottom_line = "Mairie du 6e arrondissement — 78 rue Bonaparte, 75006 Paris — Entrée libre sur présentation d’un justificatif.";
    expected_words.extend(tokenize_words(bottom_line));
    content.begin_text();
    content.set_font(Name(b"F2"), 10.0);
    content.next_line(56.7, 480.0);
    content.show(Str(&encode_win_ansi(bottom_line)));
    content.end_text();

    let p1_ftr = "Page 1 / 2";
    expected_words.extend(tokenize_words(p1_ftr));
    content.begin_text();
    content.set_font(Name(b"F1"), 9.5);
    content.next_line(268.0, 42.0);
    content.show(Str(&encode_win_ansi(p1_ftr)));
    content.end_text();

    pdf.stream(content1_id, &content.finish());

    let mut page1 = pdf.page(page1_id);
    page1.parent(page_tree_id);
    page1.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res1 = page1.resources();
    res1.fonts()
        .pair(Name(b"F1"), font_reg_id)
        .pair(Name(b"F2"), font_bold_id)
        .pair(Name(b"F3"), font_italic_id);
    res1.finish();
    page1.contents(content1_id);
    page1.finish();

    // ========================== PAGE 2 ==========================
    let mut content2 = Content::new();
    let p2_title = "PROGRAMME DÉTAILLÉ DES ACTIVITÉS ET CALENDRIER D’AUTOMNE";
    expected_words.extend(tokenize_words(p2_title));
    content2.begin_text();
    content2.set_font(Name(b"F2"), 13.5);
    content2.next_line(56.7, 780.0);
    content2.show(Str(&encode_win_ansi(p2_title)));
    content2.end_text();

    let p2_paras = [
        (
            Name(b"F1"),
            "Les inscriptions aux ateliers trimestriels sont ouvertes à l’ensemble des résidents de l’arrondissement sur simple présentation de la carte senior municipale au guichet d’accueil de la mairie.",
        ),
        (
            Name(b"F1"),
            "• Mardi 10h00 : Permanence d’accompagnement aux démarches administratives et fiscales en ligne",
        ),
        (
            Name(b"F1"),
            "• Jeudi 14h30 : Atelier pratique de bureautique, classement des courriers et édition de documents sur Mac",
        ),
        (
            Name(b"F1"),
            "• Vendredi 15h00 : Conférence d’histoire de l’art et visite commentée du patrimoine parisien",
        ),
        (
            Name(b"F3"),
            "Renseignements et réservations auprès du pôle seniors municipal au premier étage de la mairie du lundi au vendredi de 9h00 à 17h00.",
        ),
    ];

    let mut y2 = 735.0f32;
    for &(f_name, ptxt) in &p2_paras {
        expected_words.extend(tokenize_words(ptxt));
        for wrapped in wrap_line_at_chars(ptxt, 78) {
            content2.begin_text();
            content2.set_font(f_name, 10.8);
            content2.next_line(56.7, y2);
            content2.show(Str(&encode_win_ansi(&wrapped)));
            content2.end_text();
            y2 -= 14.5;
        }
        y2 -= 13.0;
    }

    let p2_ftr = "Page 2 / 2";
    expected_words.extend(tokenize_words(p2_ftr));
    content2.begin_text();
    content2.set_font(Name(b"F1"), 9.5);
    content2.next_line(268.0, 42.0);
    content2.show(Str(&encode_win_ansi(p2_ftr)));
    content2.end_text();

    pdf.stream(content2_id, &content2.finish());

    let mut page2 = pdf.page(page2_id);
    page2.parent(page_tree_id);
    page2.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res2 = page2.resources();
    res2.fonts()
        .pair(Name(b"F1"), font_reg_id)
        .pair(Name(b"F2"), font_bold_id)
        .pair(Name(b"F3"), font_italic_id);
    res2.finish();
    page2.contents(content2_id);
    page2.finish();

    pdf.pages(page_tree_id).kids([page1_id, page2_id]).count(2);
    (pdf.finish(), expected_words)
}

/// 25: Mixed-Layout Insurance Contract (`Conditions Particulières Habitation`):
/// Top 2-column Sender/Subscriber header + bottom full-width guarantee table
fn build_complex_mixed_contract_pdf() -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let font_bold_id = Ref::new(4);
    let page_id = Ref::new(5);
    let content_id = Ref::new(6);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_bold_id)
        .base_font(Name(b"Helvetica-Bold"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut content = Content::new();
    let mut expected_words = Vec::new();

    let title = "Conditions Particulières — Contrat Multirisque Habitation N° 000123";
    expected_words.extend(tokenize_words(title));
    content.begin_text();
    content.set_font(Name(b"F2"), 14.0);
    content.next_line(56.7, 780.0);
    content.show(Str(&encode_win_ansi(title)));
    content.end_text();

    let left_hdr = [
        "MAIF — Délégation Départementale de Paris",
        "Service Gestion des Contrats Sociétaires",
        "Date d’effet : 1er novembre 2026",
    ];
    let right_hdr = [
        "Sociétaire assurée :",
        "Madame Marie DUPONT",
        "Appartement 3 pièces — 75006 PARIS",
    ];
    for (i, &l) in left_hdr.iter().enumerate() {
        expected_words.extend(tokenize_words(l));
        content.begin_text();
        content.set_font(Name(b"F1"), 10.0);
        content.next_line(56.7, 740.0 - (i as f32) * 15.0);
        content.show(Str(&encode_win_ansi(l)));
        content.end_text();
    }
    for (i, &r) in right_hdr.iter().enumerate() {
        expected_words.extend(tokenize_words(r));
        content.begin_text();
        content.set_font(Name(if i == 1 { b"F2" } else { b"F1" }), 10.0);
        content.next_line(335.0, 740.0 - (i as f32) * 15.0);
        content.show(Str(&encode_win_ansi(r)));
        content.end_text();
    }

    // Full-width guarantee table
    let x_cols = [56.7f32, 290.0, 430.0, 538.0];
    let top_y = 650.0f32;
    let row_h = 25.0f32;
    let rows = [
        ("Garantie couverte", "Plafond garanti", "Franchise"),
        ("Incendie, explosion et dégâts des eaux", "85 000,00 €", "120,00 €"),
        ("Vol, vandalisme et bris de glaces", "40 000,00 €", "120,00 €"),
        ("Responsabilité civile vie privée", "Illimité", "0,00 €"),
        ("Assistance à domicile 24h/24", "Incluse", "0,00 €"),
    ];

    content.set_line_width(0.8);
    for r in 0..=rows.len() {
        let y = top_y - (r as f32) * row_h;
        content.move_to(x_cols[0], y);
        content.line_to(x_cols[3], y);
        content.stroke();
    }
    for &x in &x_cols {
        content.move_to(x, top_y);
        content.line_to(x, top_y - (rows.len() as f32) * row_h);
        content.stroke();
    }

    for (r_idx, &(c0, c1, c2)) in rows.iter().enumerate() {
        let y = top_y - (r_idx as f32) * row_h - 17.0;
        let f = if r_idx == 0 { Name(b"F2") } else { Name(b"F1") };
        for (c_idx, &txt) in [c0, c1, c2].iter().enumerate() {
            expected_words.extend(tokenize_words(txt));
            content.begin_text();
            content.set_font(f, 10.0);
            content.next_line(x_cols[c_idx] + 6.0, y);
            content.show(Str(&encode_win_ansi(txt)));
            content.end_text();
        }
    }

    let sig_line = "Cotisation annuelle TTC : 264,00 € — Fait à Paris, exemplaire destiné à l’assurée.";
    expected_words.extend(tokenize_words(sig_line));
    content.begin_text();
    content.set_font(Name(b"F2"), 11.0);
    content.next_line(56.7, 495.0);
    content.show(Str(&encode_win_ansi(sig_line)));
    content.end_text();

    pdf.stream(content_id, &content.finish());

    let mut page = pdf.page(page_id);
    page.parent(page_tree_id);
    page.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
    let mut res = page.resources();
    res.fonts()
        .pair(Name(b"F1"), font_reg_id)
        .pair(Name(b"F2"), font_bold_id);
    res.finish();
    page.contents(content_id);
    page.finish();

    pdf.pages(page_tree_id).kids([page_id]).count(1);
    (pdf.finish(), expected_words)
}

/// 26: 3-Page Financial & Savings Portfolio Statement (`Relevé Annuel de Patrimoine — 3 pages`)
fn build_complex_3page_patrimoine_pdf() -> (Vec<u8>, Vec<String>) {
    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let font_reg_id = Ref::new(3);
    let font_bold_id = Ref::new(4);
    let page1_id = Ref::new(5);
    let content1_id = Ref::new(6);
    let page2_id = Ref::new(7);
    let content2_id = Ref::new(8);
    let page3_id = Ref::new(9);
    let content3_id = Ref::new(10);

    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.type1_font(font_reg_id)
        .base_font(Name(b"Helvetica"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));
    pdf.type1_font(font_bold_id)
        .base_font(Name(b"Helvetica-Bold"))
        .encoding_predefined(Name(b"WinAnsiEncoding"));

    let mut expected_words = Vec::new();
    let x_cols = [56.7f32, 290.0, 415.0, 538.0];

    let pages_data = [
        (
            page1_id,
            content1_id,
            vec![
                ("Livret A Épargne Réglementée", "3,00 %", "22 950,00 €"),
                ("Livret de Développement Durable", "3,00 %", "12 000,00 €"),
            ],
        ),
        (
            page2_id,
            content2_id,
            vec![
                ("Plan Épargne Logement Ancien", "2,50 %", "18 400,00 €"),
                ("Contrat Assurance Vie Fonds Euros", "3,10 %", "45 200,00 €"),
            ],
        ),
        (
            page3_id,
            content3_id,
            vec![
                ("Compte Courant Postal Rémunéré", "0,50 %", "4 140,00 €"),
                ("Total général des avoirs détenus", "—", "102 690,00 €"),
            ],
        ),
    ];

    for (p_idx, (p_id, c_id, rows)) in pages_data.into_iter().enumerate() {
        let mut content = Content::new();
        let hdr = "Banque Postale Patrimoine — Synthèse Annuelle des Avoirs";
        expected_words.extend(tokenize_words(hdr));
        content.begin_text();
        content.set_font(Name(b"F2"), 10.0);
        content.next_line(56.7, 800.0);
        content.show(Str(&encode_win_ansi(hdr)));
        content.end_text();

        let top_y = 720.0f32;
        let row_h = 26.0f32;
        let total_rows = rows.len() + 1;
        content.set_line_width(0.8);
        for r in 0..=total_rows {
            let y = top_y - (r as f32) * row_h;
            content.move_to(x_cols[0], y);
            content.line_to(x_cols[3], y);
            content.stroke();
        }
        for &x in &x_cols {
            content.move_to(x, top_y);
            content.line_to(x, top_y - (total_rows as f32) * row_h);
            content.stroke();
        }

        for (c, &h) in ["Support d’épargne", "Taux annuel", "Valorisation"].iter().enumerate() {
            expected_words.extend(tokenize_words(h));
            content.begin_text();
            content.set_font(Name(b"F2"), 10.5);
            content.next_line(x_cols[c] + 6.0, top_y - 17.0);
            content.show(Str(&encode_win_ansi(h)));
            content.end_text();
        }

        for (r_idx, &(c0, c1, c2)) in rows.iter().enumerate() {
            let y = top_y - ((r_idx + 1) as f32) * row_h - 17.0;
            for (c, &txt) in [c0, c1, c2].iter().enumerate() {
                expected_words.extend(tokenize_words(txt));
                content.begin_text();
                content.set_font(Name(b"F1"), 10.5);
                content.next_line(x_cols[c] + 6.0, y);
                content.show(Str(&encode_win_ansi(txt)));
                content.end_text();
            }
        }

        let ftr = format!("Page {} / 3", p_idx + 1);
        expected_words.extend(tokenize_words(&ftr));
        content.begin_text();
        content.set_font(Name(b"F1"), 9.5);
        content.next_line(260.0, 40.0);
        content.show(Str(&encode_win_ansi(&ftr)));
        content.end_text();

        pdf.stream(c_id, &content.finish());

        let mut page = pdf.page(p_id);
        page.parent(page_tree_id);
        page.media_box(Rect::new(0.0, 0.0, 595.28, 841.89));
        let mut res = page.resources();
        res.fonts()
            .pair(Name(b"F1"), font_reg_id)
            .pair(Name(b"F2"), font_bold_id);
        res.finish();
        page.contents(c_id);
        page.finish();
    }

    pdf.pages(page_tree_id)
        .kids([page1_id, page2_id, page3_id])
        .count(3);
    (pdf.finish(), expected_words)
}
