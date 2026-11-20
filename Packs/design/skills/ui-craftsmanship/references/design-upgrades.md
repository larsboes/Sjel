# Design upgrades

Compare the same record, state and viewport before and after. Identify where the current
screen makes the reader search for the subject, a fact that affects the decision, or the next
action. For each change, state the task it makes easier and use the screen's actual data and
Sjel tokens.

When working from a reference, record the visible design choice, the task it could improve in
Sjel, and the check that would confirm the improvement. A screenshot can show where a button
sits. A live view can show whether it stays reachable while scrolling. Keep a source link when
the reference is public.

## Design elements

| Element | When and how to use it |
|---|---|
| Subject-led header | When recognizing a place, object or media item matters, pair its title with one representative image. Put the title on a stable surface when image contrast varies. Use a compact identity block on text-heavy screens or when imagery is unavailable. |
| At-a-glance facts | Pull the few facts that affect the next decision out of metadata. Pair each value with the context that makes it meaningful, such as an opening status with its closing time. Use existing status tones and keep the meaning readable in text. |
| One primary action | Give the current task's next step a clear verb and the strongest control emphasis. Keep frequently used secondary actions visible in a quieter treatment. Give a long touch view a bottom action only when it stays useful while scrolling. Place it in the safe area, reserve space below the content and verify that the final item can scroll clear of it. |
| Sections in decision order | Put the identity and decision facts before details. Arrange later sections around the questions the reader asks in sequence. Preserve discoverability when section navigation overflows or labels grow. |
| Task-based panes | On a wide surface, give the active task a stable pane and put related browsing beside it when both benefit from being visible. On a narrow surface, keep the active context available while presenting related content through native navigation or disclosure. |
| Evidence beside a review decision | For an import or generated draft, show the proposed result and its review state beside the confirmation action. Open the supporting source in place so the reader can verify a field and return to the same item. |

## Reference studies

- **Dolores Park place-detail redesign.** The before view puts a small title and several
  similarly emphasized actions above a photo grid. The redesign makes one image and title
  the opening composition, pairs the rating with its review count and the opening status
  with its closing time, and gives Directions its own bottom action. The screenshot shows
  the proposed composition. Scroll behavior and readability across photos require a live check.
- **Tidewell, Mahsa Nasiri.** The design puts sea temperature and the first booking action
  beside photography that shows the experience. Later sections answer beginner questions
  about the session and what to expect. The same sequence fits a Sjel event detail when
  the reader needs to decide whether and how to attend. [Tidewell landing page](https://dribbble.com/shots/27869157-Sea-Swimming-and-Sauna-Club-Landing-Page)
- **Music Player for iPhone Duo, Mahsa Nasiri.** The concept uses one half for the current
  player and the other for the queue, lyrics and related media. A wide Sjel Soundscape
  view can use this split to keep playback controls in reach while browsing.
  [Music Player for iPhone Duo](https://dribbble.com/shots/27799868-Music-Player-for-iPhone-Duo)
- **Fieldnote, Mahsa Nasiri.** The landing page shows the path from consent through
  transcript to a draft ready for review. In a Sjel import review, the same principle
  can put the extracted value and its source within reach of the approval action.
  [Fieldnote landing page motion](https://dribbble.com/shots/27869296-Fieldnote-AI-Therapy-Notes-Landing-Page-Motion)

## Review

Check whether the first viewport identifies the subject, answers the immediate decision
and exposes the next action. Compare representative records with long titles, missing
images and changing status. Check the last item and keyboard focus with any pinned control.
Apply the [state inventory](../SKILL.md#rule-3-the-state-inventory) and
[contrast and layout checks](mechanical-sympathy-in-ui.md) before accepting the upgrade.
