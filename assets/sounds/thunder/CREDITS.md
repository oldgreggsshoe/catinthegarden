# Thunder samples

All twelve are CC0 1.0 (public domain dedication): free to use, modify and
ship commercially, no credit required. Credit is listed anyway so the
originals can be found again. Licences were checked on each source page on
9 October 2026.

The Freesound files here are Freesound's public high-quality previews (mp3,
about 128 kbit/s). The original WAVs are free too but need a (free) Freesound
login to download; fetch them from the page if better quality is needed.

| File | Source | Author | Notes |
|---|---|---|---|
| closeup_thunder_strike_01__loganzsound_840628.mp3 | https://freesound.org/people/loganzsound/sounds/840628/ | loganzsound | close strike, crack |
| short_thunder_mid__SholeColtis_683421.mp3 | https://freesound.org/people/SholeColtis/sounds/683421/ | SholeColtis | short, mid distance |
| thunder_12__LukaCafuka_795412.mp3 | https://freesound.org/people/LukaCafuka/sounds/795412/ | LukaCafuka | single peal |
| thunder_strike__AyaDrevis_652690.mp3 | https://freesound.org/people/AyaDrevis/sounds/652690/ | AyaDrevis | strike and roll |
| thunder_5_dry__elmoustachio_476739.mp3 | https://freesound.org/people/elmoustachio/sounds/476739/ | elmoustachio | dry (no rain) |
| peal_of_thunder_distant__bastipictures_243782.mp3 | https://freesound.org/people/bastipictures/sounds/243782/ | bastipictures | distant peal |
| rolling_distant_thunder_heavy_rain__BlondPanda_778374.mp3 | https://freesound.org/people/BlondPanda/sounds/778374/ | BlondPanda | distant roll, rain under it |
| thunder_triple_layer__AppleCorey_652325.mp3 | https://freesound.org/people/AppleCorey/sounds/652325/ | AppleCorey | layered rumble |
| high_thunder_suburb_08__klankbeeld_169534.mp3 | https://freesound.org/people/klankbeeld/sounds/169534/ | klankbeeld | long, suburban background |
| three_mighty_thunders__Nimlos_522025.mp3 | https://freesound.org/people/Nimlos/sounds/522025/ | Nimlos | three big peals, long file |
| rain_long_thunder__WuxiaScrub_opengameart.ogg | https://opengameart.org/content/rain-long-thunder | WuxiaScrub | rain, long thunderclap at ~20 s |
| light_rain_distant_thunder_2016__commons.ogg | https://commons.wikimedia.org/wiki/File:Light_Rain_Distant_Thunder_July_5th_2016.wav | (Freesound upload, via Wikimedia Commons) | rain with distant thunder; converted from WAV to Ogg Vorbis q6 |

Not taken: most other Wikimedia Commons thunder recordings are CC BY or
CC BY-SA (royalty-free, but they need credit, and BY-SA requires sharing
edits under the same licence).

## clips/

What the game plays (`crates/app/src/thunder_clips.rs`): eleven single
thunder events cut from the files above, with a 30 ms fade in and a 1.5 s
fade out, mono 44.1 kHz, peak 0.9, Ogg Vorbis q5. Sorted by how fast each
rises to its loudest: `crack_*` (within 0.5 s: close strikes), `mid_*`
(0.5-1.25 s), `roll_*` (slower: far strikes). The game evens out their
loudness when it loads them.

| Clip | Cut from | At (s) |
|---|---|---|
| crack_01 | closeup_thunder_strike_01 (loganzsound) | 0.55 |
| crack_02 | three_mighty_thunders (Nimlos) | 60.7 |
| mid_01 | short_thunder_mid (SholeColtis) | 0.15 |
| mid_02 | thunder_12 (LukaCafuka) | 0.9 |
| mid_03 | three_mighty_thunders (Nimlos) | 27.7 |
| mid_04 | thunder_triple_layer (AppleCorey) | 24.65 |
| mid_05 | thunder_5_dry (elmoustachio) | 5.3 |
| roll_01 | three_mighty_thunders (Nimlos) | 95.0 |
| roll_02 | three_mighty_thunders (Nimlos) | 127.35 |
| roll_03 | thunder_strike (AyaDrevis) | 2.0 |
| roll_04 | rain_long_thunder (WuxiaScrub) | 22.7 |

Left out: peal_of_thunder_distant, high_thunder_suburb_08 and
rolling_distant_thunder_heavy_rain (thunder under 20 dB above their rain or
traffic), and light_rain_distant_thunder_2016 (thunder too faint to cut).
