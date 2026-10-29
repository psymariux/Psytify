# Brand kit di Psytify: brief e prompt

Preparato con la skill `brandkit`. In questa sessione non era disponibile un generatore di
immagini, quindi qui ci sono la strategia e i prompt pronti, da incollare in un generatore
(per esempio il tuo strumento di immagini preferito con rapporto 16:10). Le immagini non sono
state generate.

**Modo:** costruire sull'identità esistente. Marchio, carattere e palette esistono già
nell'app e non vanno ridisegnati.

## Asset esistenti da riusare

| Elemento | Valore | File |
|---|---|---|
| Marchio | Tre barre verticali arrotondate, larghe uguali, altezze crescenti 9/13/17 su 24, in lime | `client/assets/icons/mark.svg` |
| Wordmark | "Psytify" in Inter SemiBold, maiuscole come scritto | — |
| Carattere | Inter 400/500/600 | `client/assets/fonts/` |
| Fondo | `#000000`; superfici `#111111`, `#2a2a2a` | `client/src/ui/theme.rs` |
| Accento | Lime `#c6f24e`, solo su marchio e dati | — |
| Neutri | `#ffffff`, `#b4b4b4`, `#949494` | — |

## Strategia

- **Categoria:** player musicale desktop e catalogo con licenze documentate.
- **Pubblico:** chi ascolta e chi pubblica musica propria o licenziata: artisti indipendenti,
  piccole etichette, ascoltatori attenti ai diritti.
- **Promessa:** ogni brano che suona ha un titolare e una prova. Niente di preso da altre parti.
- **Personalità:** notturno, preciso, onesto, essenziale.
- **Metafora centrale:** il livello del suono che sale (le tre barre) incontra la firma: la
  licenza firmata e l'URL firmato che rende possibile l'ascolto.
- **Da evitare:** onde sonore cliché, cuffie, note musicali, neon, gradienti viola, lucidità da
  "streaming generico", glow. Lo stile dell'app non usa glass, gradienti od ombre, e il brand
  segue la stessa regola.

**Tagline:** "Ogni brano ha una firma." Gioca sul doppio senso: la licenza firmata del titolare
e l'URL firmato che apre l'ascolto.

## Prompt 1: panoramica 3 × 3 (16:10)

```
Create a premium brand-kit overview image for "Psytify".

Brand strategy:
- identity mode: build on existing brand — do not redesign the logo
- existing assets: logo mark = three vertical rounded bars of equal width and rising heights
  (short, medium, tall, ratio 9:13:17), solid lime #c6f24e; wordmark "Psytify" set in Inter
  SemiBold, white; typeface Inter; palette true black #000000, surfaces #111111 and #2a2a2a,
  white #ffffff, greys #b4b4b4 and #949494, single accent lime #c6f24e
- category: desktop music player and catalogue where every track has documented rights
- audience: independent artists, small labels, listeners who care about rights
- personality: nocturnal, precise, honest, essential
- core metaphor: rising sound level meets a signature (signed licence, signed listening link)

Canvas: 16:10, landscape.

Layout: 3×3 grid on a true-black presentation canvas, generous equal gutters, square-cornered
panels separated only by a one-step lightness change (#000 canvas, #111 panels), no borders,
no shadows, no gradients, no glow.

Panels:
1. Logo cover — the three-bar mark large, wordmark "Psytify" beside it, vast negative space.
2. Logo construction — the mark on a quiet grid showing the 9:13:17 height steps and equal
   widths, thin grey construction lines.
3. Digital application — a desktop music player bottom bar on black: small square cover,
   track title "Low Tide", artist "Marea", a white circular play button with a black triangle,
   a thin progress line whose played part is lime.
4. Brand essence — the tagline "Ogni brano ha una firma." in large white Inter, nothing else.
5. Color system — five flat rectangular swatches in a row: #000000, #111111, #2a2a2a,
   #ffffff, and a smaller lime #c6f24e chip; hex labels small and legible.
6. Typography — a large "Aa" and the alphabet in Inter at weights 400, 500, 600, white on black.
7. Physical application — a matte black vinyl sleeve with the lime mark small in one corner
   and a printed licence card beside it reading "CC BY · Marea".
8. Image direction — a night photograph of a quiet harbour with one distant lime light,
   heavy grain, black dominant, no people.
9. System detail — a row of interface pieces: a pill-shaped search field "Cerca brani o
   artisti", a white pill button "Riproduci", an outlined pill "Scarica", a track row with a
   lime title marking the song now playing.

Visual mode: dark, cultural and precise — nocturnal music brand, restrained.

Palette: true black, near-black surfaces, white, two greys, one lime accent used only on the
mark and on "now playing" data.

Style: premium, sparse, cinematic, brand-guidelines deck, sentence case, no clutter, no extra
logos, no tiny fake text, no neon, no gradients, no glass, no glow, no drop shadows.

Typography: Inter only, readable, high hierarchy.

Logo: reuse the existing three-bar mark exactly; repeat it consistently across panels.
```

## Prompt 2: mini-deck 2 × 3 (16:10)

```
Create a premium 2×3 brand mini-deck image for "Psytify", building on the existing identity
(three-bar lime mark #c6f24e with bar heights 9:13:17, wordmark "Psytify" in Inter SemiBold,
true black #000000 with #111111 panels, white and grey text). Do not redesign the logo.

Canvas: 16:10. Layout: 2 rows × 3 columns, equal gutters, square panels, no borders, no
shadows, no gradients, no glow.

1. Logo / wordmark — offset left, extremely minimal.
2. Product surface — a desktop app window on black: left sidebar with "Home", "Cerca",
   "Libreria"; an album header card with a flat dark-green square cover marked "B", the title
   "Banchi di sabbia" and a white pill button "Riproduci".
3. Functional panel — a search field reading "ma" and three result rows; the second row's
   title in lime to show it is playing.
4. Atmosphere — black-and-white grainy night photo of a seaside promenade, one lime point of
   light, no people.
5. Symbol — the three-bar mark centred inside a thin grey square frame with a small licence
   seal motif made of a single circle line.
6. Tagline — "Ogni brano ha una firma." large, white, on black.

Style: quiet, sharp, intentional, nocturnal; Inter only; minimal legible text.
```

## Uso dei risultati

- Controlla che il marchio sia identico in ogni pannello: tre barre, tre altezze crescenti,
  stesso lime. Rigenera se il generatore lo deforma.
- Gli eventuali testi finti generati in piccolo vanno coperti o rifatti: il brand usa pochi
  testi, leggibili.
- La foto del pannello "Image direction" è generata: non presentarla mai come un luogo o un
  evento reale di Psytify.
