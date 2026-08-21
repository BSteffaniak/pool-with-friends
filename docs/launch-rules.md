# PWMTF launch rules evidence

This record distinguishes confirmed Miniclip behavior from PWMTF launch-profile decisions. The executable owner is `pwmtf_game_domain`; this document records the source and interpretation used by its fixtures.

## Confirmed Miniclip behavior

Miniclip's official support article, [How to Move the Cue Ball (8 Ball Pool)](https://support.miniclip.com/hc/en-us/articles/203747476-How-to-Move-the-Cue-Ball-8-Ball-Pool), updated June 6, 2024, states:

- The breaker may move the cue ball only behind the break line.
- After an opponent foul, the incoming player may place the cue ball anywhere on the table.
- Pocketing the cue ball is an example foul.
- Cue-ball contact selection produces spin and changes direction after a rail or ball contact.

Miniclip's official [How to Play 9 Ball](https://support.miniclip.com/hc/en-us/articles/360010436313-How-to-Play-9-Ball) article explicitly identifies the following as shared with its 8-ball rules:

- Failing to contact any object ball is a foul.
- Running out the turn timer is a foul.
- Pocketing the cue ball is a foul.
- After object-ball contact, failing to pocket a legal ball or drive a ball to a rail is a foul.
- An illegal break is a foul.

## PWMTF launch profile decisions

Where Miniclip's public support material is incomplete, PWMTF version 1 uses the binding decisions in `INVARIANTS.md` and the local progress document:

- The table stays open after the break, regardless of groups pocketed during the break.
- First legally pocketed post-break group assigns solids or stripes.
- Only the 8-ball requires a called pocket.
- Pocketing the 8-ball on the break reracks rather than winning.
- A legal 8-ball requires the shooter's group to be cleared, no foul, and the called pocket to match the actual pocket.
- Any other pocketed 8-ball loses the match.
- A timeout ends only the turn, grants the opponent ball-in-hand, and can never complete a match.
- Concession is the only non-8-ball completion reason.
- A linked rematch alternates the breaker.

## Still requiring product evidence

The exact illegal-break threshold and unusual break/rail precedence are not sufficiently documented by current official Miniclip support material. They remain excluded from completion claims until observed behavior or a stronger first-party source is captured as a versioned executable fixture.
