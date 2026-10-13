# Whiteboard and handwriting capture

The core logic and data model for photographing whiteboards and handwritten notes, dropping
repeats, keeping every version and turning each kept capture into searchable notes. It also holds
[slide capture](#slide-capture) for lecture mode. The camera and the capture screen come later. The code is in `crates/zacess-core/src/capture`.

## From a photo to the ink

A photo of the same board changes from one moment to the next: the light moves, the hand shakes
and the camera picks a new exposure. Change detection therefore works on the ink alone.

- The photo is decoded from PNG or JPEG. A phone stores a photo the way its sensor saw it with a
  note of how to turn it, so that note is applied. Transparent pixels are laid on white instead of
  being read as black.
- It is read as grey and shrunk by a whole number so its longer side is at most 320 pixels.
- The board's own brightness is estimated as the brightest value within 8 shrunk pixels of each
  point. A pixel is ink when it is at least 20% darker than the board around it. A lighting
  gradient or a shadow changes slowly across the board, so it is divided out; strokes are thin, so
  they are not.
- The board itself is taken as the brightest tenth of the frame, so it is found even when someone
  covers most of it. A region whose background is below half that brightness is something dark in
  front of the board, such as the lecturer. It is marked hidden, with a margin so its edge is not
  read as a stroke. It counts as neither ink nor board.

A board darker than a sixth of full brightness, such as a blackboard, is not handled by this rule
and reads as having no ink.

## What a photo can turn out to be

| Outcome | When |
| --- | --- |
| Blank | No ink could be seen, for instance in a dim or overexposed frame. Nothing is kept, so a bad photo can never become the baseline that every later photo is compared with |
| Obscured | More than half of the frame is hidden behind something, so the photo cannot be judged |
| Repeat | The same writing as a kept capture. It is dropped and counted on the capture it repeats, with the time it was seen |
| New version | Writing was added, rubbed out or rewritten on a board seen before. The outcome names the version before it and how much ink changed. It also lists the lines added and removed, so it can be announced in words |
| New board | A board or page not seen before. Also one with most of its earlier writing gone |

Photos are only compared with boards of the same kind (whiteboard or handwritten page) and the
same size. Turning the phone from landscape to portrait therefore starts a new board.

## Repeats, versions and new boards

Comparing ink pixel by pixel fails twice over: a slightly different photo of the same writing
moves every stroke a little while a rewritten line sits exactly where the old one was. So the
comparison works on bands of writing.

| Step | What happens |
| --- | --- |
| Hide | Pixels hidden in either photo are removed from both, so a person standing in front of part of the board is not a change |
| Line up | The photo is compared with the latest version of every board of its kind, so turning back to an earlier board is recognised. The two are lined up by the shift of up to 15 shrunk pixels, about 5% of the frame, that best matches their row and column totals |
| Band | Rows with ink are grouped into bands, a line of text or a row of a drawing, with gaps of under two rows kept inside a band. Bands of fewer than six pixels or under one part in a thousand of the ink are dropped as specks |
| Match | Each band is reduced to its column profile, how much ink stands in each column, smoothed over three columns. When about as much ink stands in the other photo (between three quarters and four thirds) the bands are the same writing if the profiles correlate at least 0.8 over the columns that hold ink. When one side has much more ink, the smaller side's writing may sit inside the larger one: writing added or rubbed out beside a line is counted as just the extra ink, not the whole line |
| Measure | Ink in bands that match nothing is added. The same bands in the old photo count as removed. Each is a share of the new or the old capture's ink. Bands with at least 40 pixels of change are counted |
| Decide | Under 3% added and under 3% removed with no counted band is a repeat. A counted band is a change however full the board is, so one new line on a full board is still a new version. More than 60% of the earlier ink gone is a wiped or different board |

Restricting the correlation to inked columns matters: two different lines of the same length would
otherwise agree on their empty margins. The thresholds are settings and are checked when a photo
arrives, so a setting that would make every photo a repeat, such as an ink threshold of zero, is an
error rather than a silent failure. OCR runs only for a capture that is kept, so a lecturer who
keeps talking in front of the same board costs nothing.

## What is kept

A session holds boards, oldest first, with a schema version. A board holds its captures, each a
version numbered from 1 with:

- the time it was taken and the time the writing was last seen, both in Unix seconds
- the file and a hash of its bytes, so a moved photo can still be matched to its capture
- how it differs from the version before, as ink added and removed, the bands that changed and the
  shift that lined them up
- how many later photos repeated it
- the document OCR read from it, named after the board's first heading, else its first line cut to
  eight words, else its number
- a legibility score, the mean confidence of its words, with bullets and lone symbols left out
- the share of the ink that lies inside recognised words

Legibility alone can mislead: an engine that picks out only a neat heading from a page of joined
handwriting scores well on the words it read. The share of ink read is the other half. A
capture is readable only when both are high. A session lists the captures that are not, so a page
the engine could not read is marked rather than silently wrong.

Two versions can be compared by text: the lines added, removed and kept, with a list or table
giving a line per item or row. Lines are matched with case and spaces dropped, the letters OCR
confuses with digits (o, l, i, s, z) read as those digits and one character in five forgiven, but
only when every digit and symbol otherwise matches. A changed digit, sign or symbol is never
forgiven, because that is the change that matters, so a long formula the engine reads differently in
two photos of the same board can show as a changed line. That errs towards showing a change a
person can check against the photo.

| Need | How |
| --- | --- |
| Search | Lines holding every word of a query as a whole word or its start, ignoring case, with the board, capture and time to open |
| Ask | Every kept capture goes into the same [index](ask.md) as the rest of the library, cited by capture |
| Check | Words the engine was unsure of, by the same [thresholds](ocr.md#confidence-thresholds) as a scanned page |
| Keep | The session serialises to JSON. Ink masks are not stored; a reloaded session rebuilds them from the files when the next photo arrives. A file that cannot be read is skipped once rather than on every photo |

## Handwriting

A page of handwritten notes goes through the same OCR engine, which reads neat print and little
else. A model for joined handwriting is left for later; the readability check above is what stops
a page it could not read from passing as notes.

## Tests

The tests use sample images in `tests/fixtures/whiteboard`, described in the
[fixtures README](../tests/fixtures/README.md): a board photographed, photographed again in
different light with a small shift, with a line added, with a line rubbed out and rewritten, then
a wiped board with the next topic, plus a page of notes on lined paper. A sidecar says which
board each belongs to and whether it is new, a repeat or a change. Change detection is tested
without an OCR engine, along with blank and obscured photos, bad settings, a reloaded session and
the rule that photos of different kinds never share a board. Smaller tests cover a person standing
in front of the board, writing added beside a line, one new line on a full board, transparent
pixels and the line matching rules. The test that reads the boards, compares versions by text,
searches and asks a question runs where Tesseract is installed, as the OCR tests do.

## Slide capture

Lecture mode watches a projector through the phone's camera. The core turns a stream of frames into
the slides of a lecture. The camera, the frame rate and the screen are the app's. The code is in
`crates/zacess-core/src/capture/slides.rs`.

### Done when

The core of the issue is done when all of these hold. A real phone and projector are the one item
that cannot be checked here.

1. Every frame gets a perceptual hash, which is the same for the same photograph and survives a
   change of exposure, a few pixels of camera shake and a soft focus.
2. A frame is judged the same slide, a step of the same slide or a different slide, with
   thresholds that can be tuned. Settings that would make every frame a repeat or every frame new
   are refused.
3. Of a run of repeats the clearest frame is the one kept. A long run cannot drift away from
   the slide it started as.
4. A slide that is only partly changed, a bullet appearing or an animation step, is a new step of
   the same slide and not a new slide. The slide keeps every step and the last is the one to read.
5. A different slide is a new slide. Going back to an earlier slide is a return to it and not a new
   slide.
6. A frame caught half way through a transition is not kept as a slide. A lens cap or a blank wall
   is ignored. The last slide of a lecture is kept even if only one frame of it was taken.
7. Each kept slide is placed on the [lecture timeline](timeline.md) with its steps, the time it
   first appeared and the time the lecturer last came back to it.
8. The tests use real image files: exact repeats, small shake, a change of light, soft focus, a
   slide built in three steps, genuinely new slides, a fade and a lens cap.
9. Capturing frames from a real camera pointed at a real projector is checked by a person with the
   hardware (issue 9).

### How a frame is judged

| Step | What happens |
| --- | --- |
| Measure | The frame is read, turned the way the phone says and shrunk to 640 pixels on its long side. Its sharpness is the variance of its Laplacian and its contrast the spread of its brightness |
| Hash | A grid of 17 by 16 area means gives 256 bits, each saying which way brightness falls to the next cell on its row. A bit counts only where the fall is large against the spread of the whole grid, so a change of exposure leaves the same bits in play. The hash is the bits and which of them count |
| Compare | The distance is the share of counted bits on which two hashes disagree. At or below 0.01 the frames are the same state, once the hash has enough structure to say so. At or above 0.30 they are different slides. Between the two the frames are compared by content |
| Content | Where the frame has structure whatever its colours: pixels at an edge after a light blur. Tall straight lines, such as the edge of the projection, are left out so they do not join every line of text into one band. The mask is four times smaller than the frame, so a line of text becomes a solid band |
| Bands | The bands of content are compared as for [boards](#repeats-versions-and-new-boards): the frames are lined up by the shift that matches best, up to 8 cells. Each band is then matched by its column profile. A change of up to 15% of the content with no whole line added or taken away is a repeat. More than 60% of the earlier content gone is a different slide. Anything between is a step of the same slide |
| Keep | A change is held until two frames in a row agree on it. A frame caught in a fade never agrees with the next and is dropped. When a repeat is sharper than the kept frame by 5% it replaces it |

A frame is compared with the latest step of every slide and every step of the slide on screen, and
the closest wins. A frame that matches another slide or an earlier step of the slide on screen is a return and is recorded with its time. The
slide's hash and content are those of the first frame to show that state, so a run of near repeats
cannot wander off. When a session is loaded again the content is rebuilt from the files.

### Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `duplicate_below` | 0.01 | Hash distance at or below which frames are the same state |
| `new_above` | 0.30 | Hash distance at or above which frames are different slides |
| `near_duplicate_below` | 0.05 | When the content cannot be compared, closer than this is a repeat |
| `edge_floor` | 10 | Grey levels of edge that count as content |
| `mask_pool` | 4 | How much coarser than the frame the content is looked at |
| `change` | see text | Band comparison: repeat under 15%, new slide over 60% gone, shift up to 8 cells, a band of 200 cells that appears or goes is a change |
| `min_contrast` | 4 | A frame flatter than this is a lens cap |
| `max_steps` | 30 | Steps one slide may hold. Past this a build is taken as a repeat of the last step, so an animation that never settles cannot fill the disk |
| `confirm_frames` | 2 | Frames in a row that must agree before a change is kept |
| `clearer_by` | 1.05 | How much sharper a frame must be to replace the kept one |

These came from a sweep over the fixtures: every build and every different slide was told apart and
10 of 11 same-state pairs were right. The one miss compares a shaken and turned frame with a dimmed
frame that has a gradient when neither is the first frame of its state. Against the first frame the
hash settles it. They are starting points to tune on a real projector.

### What is kept

A session holds slides in the order they first appeared. A slide holds its title, its steps and the
times the lecturer returned to it. A step is the clearest frame of one state of the slide, with the
times it was first and last seen, how many frames showed it, its sharpness and, once read, its text.
Reading the slides runs OCR on each kept frame and titles each slide from its last step. The session
serialises to JSON.

On the timeline each step is a mark of kind slide, the first being version 1 and each build adding
one. Slide and frame ids start at 1,000,000 so they never collide with a board's. A slide is last
seen when the lecturer last came back to it. Marks of a slide, version 1, are the signals for a new
chapter. Boards and slides can be placed on the same timeline together.

### Limits

- Tesseract reads dark text on a light ground. Light text on a coloured bar, such as a slide title,
  is not read, so a slide's title falls back to the first line that was.
- Light text on a coloured ground cannot be read by OCR, so such a slide is judged by its look alone.
- A fade frame at the very end of a lecture is kept, as there is no later frame to disagree with it.
- Two slides with the same layout and lines of the same length, differing only in the words, can be
  taken for steps of one slide. Different line lengths separate them.
- An animation that moves content without adding or removing any, such as a slide that fades from
  one image to another, may look like a repeat.
- The tests use drawn slides photographed by a simulated camera. Real projectors flicker, bands
  roll across the frame and the keystone effect skews the slide, none of which is simulated.

## Left for the interface

The camera, taking a photo on a timer or a tap, the capture screen with its repeat and new version
feedback, the version view and the timeline that lines captures up with the transcript and notes.
The [timeline](timeline.md) already places each capture on a lecture recording's clock.
