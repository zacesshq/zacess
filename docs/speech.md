# On-device transcription and live captions

The core logic and data model for transcribing lecture audio on the device and showing live
captions, with nothing uploaded. The interface, the microphone and the recording screen come later.
The code is in `crates/zacess-core/src/speech`; it is separate from `maths::speech`, which reads
equations aloud.

## Nothing leaves the device

A recogniser sits behind the `Transcriber` trait and says where it runs. The pipeline refuses one
that would run anywhere but the device, for a whole recording and for live captions alike, so a
cloud recogniser cannot be plugged in by mistake. The transcript records which recogniser and model
produced it and that it ran on the device, in the same provenance record other helpers use.

The first recogniser is [whisper.cpp](https://github.com/ggml-org/whisper.cpp), which runs the
Whisper models on the processor of a laptop or a phone. Like Tesseract and liblouis it is found at
run time: the `whisper-cli` command on the path, resolved to the file that will receive the audio
and named in the provenance, plus a model file named by `ZACESS_WHISPER_MODEL`. When either is
missing there is no recogniser and the app says so rather than sending audio away. Each chunk is
written to a temporary 16 kHz WAV, recognised and read back from whisper.cpp's full JSON output,
which carries a probability for every token. Tokens are joined into words and a word takes the
probability of its least certain piece. When no language is given the command is told to listen
for one, because its own default is to assume English. The language it settles on goes into the
transcript.

The command loads its model on every call, so it suits whole recordings and final captions. Interim
captions every second or two want an engine that stays resident, which the trait allows and which
is why interim refreshes can be turned off.

## From audio to captions

| Step | What happens |
| --- | --- |
| Decode | WAV files of 8 to 32 bits and any channel count are read with `hound` and averaged to mono. Live audio arrives as samples at any rate, which must not change mid-stream |
| Resample | Everything is brought to 16 kHz by linear interpolation. The live resampler keeps its place between batches, so buffer sizes that do not divide evenly cannot make the clock drift |
| Find speech | An energy detector learns the noise floor from the quietest fifth of the frames and marks a 20 ms frame as speech when it rises 12 dB above the floor. Gaps under half a second stay inside one utterance, utterances under 150 ms are dropped and each is padded by 100 ms so word edges survive; padded spans never overlap. Live, the floor drops quickly to meet a quieter room and rises only slowly, so a long unbroken stretch of speech is never taken for the background and the cost per frame stays constant through a long lecture |
| Chunk | Speech longer than eight seconds is cut at the quietest moment within a second of the even split, so a cut falls between words rather than inside one |
| Recognise | Each chunk goes to the recogniser with its position in the recording; the times that come back are shifted into place |
| Caption | Text becomes captions of at most two lines of 37 characters. A long utterance becomes several captions in turn, the time shared out by characters, rather than one that overflows. A caption is held long enough to read at 180 words a minute and never less than 1.5 seconds unless the next one is already due. Captions never overlap |

Live captions work frame by frame and time is derived from a count of frames, never accumulated,
so it does not drift. An utterance opens with the frames of silence just before it, so the soft
start of its first word is heard. While someone keeps talking an interim caption is refreshed every
1.5 seconds and marked as such, because its text may still change; lines already shown stay where
they are and the event says how many lines are unchanged from the previous interim caption, so
the interface redraws only the rest. When the
detector hears a pause of half a second, the utterance is recognised once more and a final caption
is issued; final text goes into the transcript and does not change. A lone click or cough never
reaches the recogniser, which could otherwise invent words for it. An utterance that reaches eight
seconds is cut at its quietest recent moment and the rest carries on as the next one. The
interface must show interim and final text differently, never by colour alone; the tagged events
carry what it needs. Recognition runs inside the call that feeds audio, so the stream is fed from
a thread of its own, never from the audio callback. A stream can resume at a known time with the
transcript so far, after a pause or a restart, so both halves share one timeline.

## Caption lines

Lines break where a reader would pause: after a sentence end first, then after a comma, semicolon
or colon, then before a joining word such as "and" or "which" and only then wherever the line is
full. A break is only taken when it leaves a reasonable share of the line, so a sentence is not cut
to two words and a long tail. A number is never parted from the unit after it, so "150 Hz" stays
on one line. Lines are at most 37 characters, following the broadcast subtitle guidelines; both
limits are settings.

## The transcript

A transcript carries a schema version, the file it describes when it came from one, its language,
its length, its provenance and a list of segments: the stretches between pauses, usually a sentence
or two, each with a start and end time, its text and its words. Every word has a time and, when
the recogniser measured one, a confidence; a word with none is never passed off as certain. A word
is reported as uncertain by the same [thresholds](ocr.md#confidence-thresholds) as OCR text, so a
number or the unit after one must be surer than an ordinary word. The interface marks it the way
it marks uncertain OCR. A person can confirm a word or correct it; a corrected word keeps the
recognised text one step away, as a run of OCR text does. Checked words are not flagged again.
A recogniser that gives no word times gets words spread evenly across the segment.

| Need | How |
| --- | --- |
| Tap a sentence to jump | Segments split into sentences at sentence punctuation using the word times, each with its own start |
| Search | Segments containing every word of a query, ignoring case, with the time of the first matching word to jump to |
| Follow along | The segment being spoken at a time; a pause belongs to the sentence before it |
| Mark what needs checking | Every uncertain word with its segment |
| Captions for players | WebVTT and SubRip, built from the same caption schedule |
| Keep | The transcript serialises to JSON; the [timeline](timeline.md) keeps it beside its recording; the library entry comes with the app |

## Tests

The tests use a synthetic sample in `tests/fixtures/audio`, described in the
[fixtures README](../tests/fixtures/README.md): four sentences spoken as three utterances with
silences between them and a sidecar giving each utterance's text and exact times. They check that
the file decodes and resamples as the sidecar says, that speech is found where the utterances are,
that captions arrive utterance by utterance with interim refreshes while feeding the file in 100 ms
batches, that a lone click is never recognised and a long utterance is cut at its quietest moment,
that the transcript can be searched, split into sentences, corrected and jumped through and that a
recogniser which would upload is refused. A scripted recogniser that knows the sidecar stands in
for recognition. The one test that runs whisper.cpp itself is skipped where it is not installed.

## Left for the interface

The microphone, the recording and playback screens, showing interim and final captions, the
transcript view with search and tap to jump, correcting uncertain words and lining the transcript up
with slides and notes on one timeline.
