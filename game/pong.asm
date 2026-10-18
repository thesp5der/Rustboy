; ============================================================================
; PONG for the Game Boy - a homebrew test game for the RustBoy emulator.
;
;   D-pad up/down : move your paddle (left side)
;   Start / A     : start the game
;   First to 7 wins.
;
; Exercises: VBlank interrupts + HALT, joypad, OAM DMA (from HRAM), sprites,
; background tiles, LCD on/off, and all four sound channels:
;   CH1 square + sweep (paddle hit, win/lose jingles)
;   CH2 square         (wall bounce, menu blip)
;   CH3 wave           (title/game-over melody)
;   CH4 noise          (point scored)
; Build: ./build.sh   (needs RGBDS)
; ============================================================================

; ---- hardware registers -----------------------------------------------------
DEF rP1    EQU $FF00
DEF rDIV   EQU $FF04
DEF rIF    EQU $FF0F
DEF rNR10  EQU $FF10
DEF rNR11  EQU $FF11
DEF rNR12  EQU $FF12
DEF rNR13  EQU $FF13
DEF rNR14  EQU $FF14
DEF rNR21  EQU $FF16
DEF rNR22  EQU $FF17
DEF rNR23  EQU $FF18
DEF rNR24  EQU $FF19
DEF rNR30  EQU $FF1A
DEF rNR32  EQU $FF1C
DEF rNR33  EQU $FF1D
DEF rNR34  EQU $FF1E
DEF rNR41  EQU $FF20
DEF rNR42  EQU $FF21
DEF rNR43  EQU $FF22
DEF rNR44  EQU $FF23
DEF rNR50  EQU $FF24
DEF rNR51  EQU $FF25
DEF rNR52  EQU $FF26
DEF rWAVE  EQU $FF30
DEF rLCDC  EQU $FF40
DEF rLY    EQU $FF44
DEF rDMA   EQU $FF46
DEF rBGP   EQU $FF47
DEF rOBP0  EQU $FF48
DEF rIE    EQU $FFFF

DEF LCD_ON EQU %10010011   ; LCD on, tiles at $8000, BG map $9800, sprites on, BG on

; ---- joypad bits (after ReadKeys) ---------------------------------------------
DEF PAD_A     EQU $01
DEF PAD_START EQU $08
DEF PAD_UP    EQU $40
DEF PAD_DOWN  EQU $80

; ---- game constants -------------------------------------------------------------
DEF STATE_TITLE EQU 0
DEF STATE_PLAY  EQU 1
DEF STATE_OVER  EQU 2
DEF PADDLE_H    EQU 24
DEF PLAYER_X    EQU 8
DEF CPU_X       EQU 144
DEF TOP         EQU 16          ; first playfield row (pixels)
DEF BOTTOM      EQU 136         ; first pixel row of the bottom wall
DEF WIN_SCORE   EQU 7
DEF BALL_SPEED  EQU 2

; ---- text: map characters to tile numbers ---------------------------------------
CHARMAP " ", 0
CHARMAP "0", 4
CHARMAP "1", 5
CHARMAP "2", 6
CHARMAP "3", 7
CHARMAP "4", 8
CHARMAP "5", 9
CHARMAP "6", 10
CHARMAP "7", 11
CHARMAP "8", 12
CHARMAP "9", 13
CHARMAP "A", 14
CHARMAP "E", 15
CHARMAP "G", 16
CHARMAP "I", 17
CHARMAP "L", 18
CHARMAP "N", 19
CHARMAP "O", 20
CHARMAP "P", 21
CHARMAP "R", 22
CHARMAP "S", 23
CHARMAP "T", 24
CHARMAP "U", 25
CHARMAP "W", 26
CHARMAP "Y", 27

; ---- memory -----------------------------------------------------------------------
SECTION "oam_shadow", WRAM0[$C000]
wOam: ds 160                    ; must be page-aligned for OAM DMA

SECTION "vars", WRAM0[$C100]
wState:      ds 1
wPlayerY:    ds 1
wCpuY:       ds 1
wBallX:      ds 1
wBallY:      ds 1
wBallDX:     ds 1
wBallDY:     ds 1
wScoreP:     ds 1
wScoreC:     ds 1
wServe:      ds 1
wCpuAim:     ds 1        ; where on the ball the CPU aims (signed px), re-rolled on each player hit
wPlayerAim:  ds 1        ; same, for the AUTOPLAY test build
wMusicPtr:   ds 2
wMusicTimer: ds 1

SECTION "hram", HRAM
hOamDma:  ds 10                 ; copy of DmaRoutine (OAM DMA must run from HRAM)
hVBlank:  ds 1
hKeys:    ds 1
hNew:     ds 1

; ---- interrupt vector + header -----------------------------------------------------
SECTION "vblank_vec", ROM0[$0040]
    jp VBlankHandler

SECTION "entry", ROM0[$0100]
    nop
    jp Start
    ds $150 - @, 0              ; rgbfix fills in the header

SECTION "main", ROM0[$0150]

VBlankHandler:
    push af
    ld a, 1
    ldh [hVBlank], a
    pop af
    reti

Start:
    di
    ld sp, $FFFE
    call WaitVBlank
    xor a
    ldh [rLCDC], a              ; LCD off (only legal during VBlank)
    call LoadTiles
    call ClearMap
    call ClearOam
    call CopyDmaRoutine

    ld a, %11100100
    ldh [rBGP], a
    ldh [rOBP0], a

    ; sound: power on, full volume, all channels to both speakers
    ld a, $80
    ldh [rNR52], a
    ld a, $77
    ldh [rNR50], a
    ld a, $FF
    ldh [rNR51], a
    call LoadWave

    ld a, STATE_TITLE
    ld [wState], a
    call ShowTitle

    ld a, %00000001
    ldh [rIE], a                ; VBlank interrupt only
    xor a
    ldh [rIF], a
    ei

MainLoop:
    xor a
    ldh [hVBlank], a
.wait:
    halt
    nop
    ldh a, [hVBlank]
    and a
    jr z, .wait

    ; ---- we are at the start of VBlank ----
    call hOamDma
    ld a, [wState]
    cp STATE_PLAY
    call z, DrawScore
    call ReadKeys

    ld a, [wState]
    cp STATE_PLAY
    jr z, .play
    cp STATE_TITLE
    jr z, .title
    call OverFrame
    jr MainLoop
.title:
    call TitleFrame
    jr MainLoop
.play:
    call PlayFrame
    ld a, [wState]              ; PlayFrame may have ended the game
    cp STATE_PLAY
    call z, BuildOam
    jr MainLoop

; ---- helpers ------------------------------------------------------------------------
WaitVBlank:
    ldh a, [rLY]
    cp 144
    jr c, WaitVBlank
    ret

CopyDmaRoutine:
    ld hl, DmaRoutine
    ld de, hOamDma
    ld b, DmaEnd - DmaRoutine
.l: ld a, [hl+]
    ld [de], a
    inc de
    dec b
    jr nz, .l
    ret

DmaRoutine:
    ld a, HIGH(wOam)
    ldh [rDMA], a
    ld a, 40
.w: dec a
    jr nz, .w
    ret
DmaEnd:

; hl = dest, de = src, bc = length
CopyMem:
.l: ld a, [de]
    inc de
    ld [hl+], a
    dec bc
    ld a, b
    or c
    jr nz, .l
    ret

LoadTiles:
    ld hl, $8000
    ld de, Tiles
    ld bc, TilesEnd - Tiles
    call CopyMem
    ; the 1bpp font is expanded to 2bpp (colour 3) as it is copied
    ld hl, $8040
    ld de, Font
    ld bc, FontEnd - Font
.f: ld a, [de]
    inc de
    ld [hl+], a
    ld [hl+], a
    dec bc
    ld a, b
    or c
    jr nz, .f
    ret

LoadWave:                       ; a triangle wave; CH3 must be off while RAM is written
    ld hl, rWAVE
    ld de, WaveData
    ld b, 16
.l: ld a, [de]
    inc de
    ld [hl+], a
    dec b
    jr nz, .l
    ret

ClearMap:
    ld hl, $9800
    ld bc, $0400
.l: xor a
    ld [hl+], a
    dec bc
    ld a, b
    or c
    jr nz, .l
    ret

ClearOam:
    ld hl, wOam
    ld b, 160
    xor a
.l: ld [hl+], a
    dec b
    jr nz, .l
    ret

; hl = destination in tile map, de = string ending in $FF
PrintStr:
.l: ld a, [de]
    cp $FF
    ret z
    ld [hl+], a
    inc de
    jr .l

ReadKeys:
    ld a, $20                   ; select the D-pad
    ldh [rP1], a
    ldh a, [rP1]
    ldh a, [rP1]
    cpl
    and $0F
    swap a
    ld b, a
    ld a, $10                   ; select the buttons
    ldh [rP1], a
    ldh a, [rP1]
    ldh a, [rP1]
    ldh a, [rP1]
    ldh a, [rP1]
    cpl
    and $0F
    or b
    ld b, a
    ldh a, [hKeys]
    xor b
    and b                       ; bits that just went from released to pressed
    ldh [hNew], a
    ld a, b
    ldh [hKeys], a
    ld a, $30
    ldh [rP1], a
    ret

; ---- screens ---------------------------------------------------------------------------
ShowTitle:
    xor a
    ldh [rLCDC], a
    call ClearMap
    call ClearOam
    call hOamDma
    ld hl, $9800 + 32 * 5 + 8
    ld de, StrPong
    call PrintStr
    ld hl, $9800 + 32 * 10 + 4
    ld de, StrPress
    call PrintStr
    ld a, LCD_ON
    ldh [rLCDC], a
    call StartMusic
    ld a, STATE_TITLE
    ld [wState], a
    ret

ShowCourt:
    xor a
    ldh [rLCDC], a
    call ClearMap
    ld hl, $9800 + 32 * 1       ; top wall
    ld b, 20
.t: ld [hl], 1
    inc hl
    dec b
    jr nz, .t
    ld hl, $9800 + 32 * 17      ; bottom wall
    ld b, 20
.b: ld [hl], 1
    inc hl
    dec b
    jr nz, .b
    ld hl, $9800 + 32 * 2 + 10  ; dashed centre line
    ld b, 15
.d: ld [hl], 3
    ld de, 32
    add hl, de
    dec b
    jr nz, .d
    ld a, LCD_ON
    ldh [rLCDC], a
    ret

; a = 1 if the player won, 0 if the CPU won
ShowOver:
    push af
    xor a
    ldh [rLCDC], a
    call ClearMap
    call ClearOam
    call hOamDma
    pop af
    and a
    jr z, .lose
    ld hl, $9800 + 32 * 6 + 6
    ld de, StrWin
    call PrintStr
    call SfxWin
    jr .common
.lose:
    ld hl, $9800 + 32 * 6 + 6
    ld de, StrLose
    call PrintStr
    call SfxLose
.common:
    ld hl, $9800 + 32 * 10 + 4
    ld de, StrPress
    call PrintStr
    ld a, LCD_ON
    ldh [rLCDC], a
    call StartMusic
    ld a, STATE_OVER
    ld [wState], a
    ret

DrawScore:
    ld a, [wScoreP]
    add 4
    ld [$9800 + 8], a
    ld a, [wScoreC]
    add 4
    ld [$9800 + 11], a
    ret

; ---- state handlers ---------------------------------------------------------------------
TitleFrame:
OverFrame:
    call MusicTick
    ldh a, [hNew]
    and PAD_START | PAD_A
    ret z
    ; fall through

StartGame:
    xor a
    ld [wScoreP], a
    ld [wScoreC], a
    ld a, 64
    ld [wPlayerY], a
    ld [wCpuY], a
    xor a
    ld [wCpuAim], a
    ld [wPlayerAim], a
    ld a, -BALL_SPEED
    ld [wBallDX], a
    call StopMusic
    call SfxWall
    call ShowCourt
    call ResetBall
    ld a, STATE_PLAY
    ld [wState], a
    ret

ResetBall:
    ld a, 76
    ld [wBallX], a
    ld a, 72
    ld [wBallY], a
    ldh a, [rDIV]               ; cheap randomness for the serve angle: -1..2
    and 3
    sub 1
    ld [wBallDY], a
    ld a, 45
    ld [wServe], a              ; frames to wait before serving
    ret

PlayFrame:
    ; ---- player paddle ----
IF DEF(AUTOPLAY)
    ; test build (./build.sh autoplay): the "player" tracks the ball by itself
    ld a, [wPlayerAim]
    ld d, a
    ld a, [wBallY]
    add 4
    add d
    ld d, a                     ; aim point
    ld a, [wPlayerY]
    add 12
    ld e, a                     ; paddle centre
    add 3
    cp d
    ld a, PAD_DOWN
    jr c, .apDone
    ld a, d
    add 3
    cp e
    ld a, PAD_UP
    jr c, .apDone
    xor a
.apDone:
    ld b, a
ELSE
    ldh a, [hKeys]
    ld b, a
ENDC
    ld a, [wPlayerY]
    bit 6, b                    ; up
    jr z, .noUp
    sub 2
    cp TOP
    jr nc, .noUp
    ld a, TOP
.noUp:
    bit 7, b                    ; down
    jr z, .noDown
    add 2
    cp BOTTOM - PADDLE_H + 1
    jr c, .noDown
    ld a, BOTTOM - PADDLE_H
.noDown:
    ld [wPlayerY], a

    ; ---- CPU paddle: creeps 1px/frame toward its aim point on the ball ----
    ld a, [wCpuAim]
    ld b, a
    ld a, [wBallY]
    add 4
    add b
    ld b, a                     ; aim point
    ld a, [wCpuY]
    add 12
    ld c, a                     ; paddle centre
    add 3
    cp b
    jr nc, .notBelow            ; paddle centre + 3 >= ball centre
    ld a, [wCpuY]
    inc a
    cp BOTTOM - PADDLE_H + 1
    jr c, .storeCpu
    ld a, BOTTOM - PADDLE_H
    jr .storeCpu
.notBelow:
    ld a, b
    add 3
    cp c
    jr nc, .cpuDone             ; ball centre + 3 >= paddle centre
    ld a, [wCpuY]
    dec a
    cp TOP
    jr nc, .storeCpu
    ld a, TOP
.storeCpu:
    ld [wCpuY], a
.cpuDone:

    ; ---- serve delay ----
    ld a, [wServe]
    and a
    jr z, BallStep
    dec a
    ld [wServe], a
    ret

BallStep:
    ld a, [wBallDX]
    ld b, a
    ld a, [wBallX]
    add b
    ld [wBallX], a
    ld a, [wBallDY]
    ld b, a
    ld a, [wBallY]
    add b
    ld [wBallY], a

    ; ---- bounce off top / bottom walls ----
    ld a, [wBallY]
    cp TOP
    jr nc, .topOk
    ld a, TOP
    ld [wBallY], a
    call NegDY
    call SfxWall
    jr .wallsDone
.topOk:
    cp BOTTOM - 8 + 1
    jr c, .wallsDone
    ld a, BOTTOM - 8
    ld [wBallY], a
    call NegDY
    call SfxWall
.wallsDone:

    ld a, [wBallDX]
    bit 7, a
    jr z, .movingRight

    ; ---- moving left: player paddle ----
    ld a, [wBallX]
    cp PLAYER_X + 8
    jr nc, .noPlayerHit         ; hasn't reached the paddle yet
    cp PLAYER_X + 2
    jr c, .noPlayerHit          ; already past it
    ld a, [wBallY]
    ld b, a
    ld a, [wPlayerY]
    ld c, a
    call PaddleOverlap
    jr nc, .noPlayerHit
    call DyFromOffset
    ld [wBallDY], a
    ld a, BALL_SPEED
    ld [wBallDX], a
    ld a, PLAYER_X + 8
    ld [wBallX], a
    ldh a, [rDIV]               ; CPU picks a new aim: -14..17 px from the ball centre.
    and 31                      ; Off-centre returns give steep angles and the odd whiff,
    sub 14                      ; so rallies end and the CPU is beatable.
    ld [wCpuAim], a
    jp SfxHit
.noPlayerHit:
    ld a, [wBallX]
    cp 4
    ret nc
    ld hl, wScoreC              ; CPU scores
    inc [hl]
    call SfxScore
    ld a, [wScoreC]
    cp WIN_SCORE
    jp nz, ResetBall
    xor a
    jp ShowOver

.movingRight:
    ld a, [wBallX]
    cp CPU_X - 7
    jr c, .noCpuHit
    cp CPU_X - 1
    jr nc, .noCpuHit
    ld a, [wBallY]
    ld b, a
    ld a, [wCpuY]
    ld c, a
    call PaddleOverlap
    jr nc, .noCpuHit
    call DyFromOffset
    ld [wBallDY], a
    ld a, -BALL_SPEED
    ld [wBallDX], a
    ld a, CPU_X - 8
    ld [wBallX], a
IF DEF(AUTOPLAY)
    ldh a, [rDIV]
    and 31
    sub 14
    ld [wPlayerAim], a
ENDC
    jp SfxHit
.noCpuHit:
    ld a, [wBallX]
    cp 152
    ret c
    ld hl, wScoreP              ; player scores
    inc [hl]
    call SfxScore
    ld a, [wScoreP]
    cp WIN_SCORE
    jp nz, ResetBall
    ld a, 1
    jp ShowOver

NegDY:
    ld a, [wBallDY]
    cpl
    inc a
    ld [wBallDY], a
    ret

; in: B = ball Y, C = paddle Y.  out: carry = overlap, A = (ball Y - paddle Y + 7)
PaddleOverlap:
    ld a, b
    sub c
    add 7
    cp 31
    ret

; in: A = hit offset from PaddleOverlap.  out: A = new vertical speed
DyFromOffset:
    cp 7
    jr c, .m2
    cp 12
    jr c, .m1
    cp 19
    jr c, .zero
    cp 24
    jr c, .p1
    ld a, 2
    ret
.m2:   ld a, -2
    ret
.m1:   ld a, -1
    ret
.zero: xor a
    ret
.p1:   ld a, 1
    ret

BuildOam:
    ld hl, wOam
    ld a, [wPlayerY]
    add 16
    ld b, a
    ld c, PLAYER_X + 8
    call PutPaddle
    ld a, [wCpuY]
    add 16
    ld b, a
    ld c, CPU_X + 8
    call PutPaddle
    ld a, [wBallY]              ; ball
    add 16
    ld [hl+], a
    ld a, [wBallX]
    add 8
    ld [hl+], a
    ld a, 2
    ld [hl+], a
    xor a
    ld [hl+], a
    ret

; hl = OAM pointer, b = Y, c = X: three stacked 8x8 sprites
PutPaddle:
    ld d, 3
.l: ld a, b
    ld [hl+], a
    ld a, c
    ld [hl+], a
    ld a, 1
    ld [hl+], a
    xor a
    ld [hl+], a
    ld a, b
    add 8
    ld b, a
    dec d
    jr nz, .l
    ret

; ---- music (CH3 wave channel) ---------------------------------------------------------------
StartMusic:
    ld hl, Melody
    ld a, l
    ld [wMusicPtr], a
    ld a, h
    ld [wMusicPtr + 1], a
    xor a
    ld [wMusicTimer], a
    ld a, $80
    ldh [rNR30], a              ; DAC on
    ld a, $20
    ldh [rNR32], a              ; 100% volume
    ret

StopMusic:
    xor a
    ldh [rNR30], a              ; DAC off silences the channel
    ret

MusicTick:
    ld a, [wMusicTimer]
    and a
    jr z, .next
    dec a
    ld [wMusicTimer], a
    ret
.next:
    ld a, [wMusicPtr]
    ld l, a
    ld a, [wMusicPtr + 1]
    ld h, a
    ld a, [hl+]
    ld e, a
    ld a, [hl+]
    ld d, a
    or e
    jr nz, .play
    ld hl, Melody               ; end marker: loop
    ld a, [hl+]
    ld e, a
    ld a, [hl+]
    ld d, a
.play:
    ld a, [hl+]
    ld [wMusicTimer], a
    ld a, l
    ld [wMusicPtr], a
    ld a, h
    ld [wMusicPtr + 1], a
    ld a, e
    ldh [rNR33], a
    ld a, d
    or $80                      ; trigger
    ldh [rNR34], a
    ret

; ---- sound effects ------------------------------------------------------------------------------
SfxHit:                         ; CH1: short high blip (880 Hz)
    xor a
    ldh [rNR10], a
    ld a, %10000000
    ldh [rNR11], a
    ld a, $F1
    ldh [rNR12], a
    ld a, $6B
    ldh [rNR13], a
    ld a, $87
    ldh [rNR14], a
    ret

SfxWall:                        ; CH2: lower blip (440 Hz)
    ld a, %10000000
    ldh [rNR21], a
    ld a, $F1
    ldh [rNR22], a
    ld a, $D6
    ldh [rNR23], a
    ld a, $86
    ldh [rNR24], a
    ret

SfxScore:                       ; CH4: noise burst
    xor a
    ldh [rNR41], a
    ld a, $F7
    ldh [rNR42], a
    ld a, $54
    ldh [rNR43], a
    ld a, $80
    ldh [rNR44], a
    ret

SfxWin:                         ; CH1: rising sweep
    ld a, $35
    ldh [rNR10], a
    ld a, %10000000
    ldh [rNR11], a
    ld a, $F4
    ldh [rNR12], a
    xor a
    ldh [rNR13], a
    ld a, $86
    ldh [rNR14], a
    ret

SfxLose:                        ; CH1: falling sweep
    ld a, $3D
    ldh [rNR10], a
    ld a, %10000000
    ldh [rNR11], a
    ld a, $F6
    ldh [rNR12], a
    xor a
    ldh [rNR13], a
    ld a, $87
    ldh [rNR14], a
    ret

; ---- data ---------------------------------------------------------------------------------------------
StrPong:  db "PONG", $FF
StrPress: db "PRESS START", $FF
StrWin:   db "YOU WIN", $FF
StrLose:  db "YOU LOSE", $FF

WaveData:   ; triangle
    db $01, $23, $45, $67, $89, $AB, $CD, $EF, $FE, $DC, $BA, $98, $76, $54, $32, $10

MACRO Note                      ; CH3 frequency register value, length in frames
    dw \1
    db \2
ENDM

Melody:
    Note $0706, 9               ; C4
    Note $0739, 9               ; E4
    Note $0759, 9               ; G4
    Note $0783, 9               ; C5
    Note $0759, 9               ; G4
    Note $0739, 9               ; E4
    Note $0706, 9               ; C4
    Note $0739, 9               ; E4
    Note $0721, 9               ; D4
    Note $0744, 9               ; F4
    Note $076B, 9               ; A4
    Note $0790, 9               ; D5
    Note $076B, 9               ; A4
    Note $0744, 9               ; F4
    Note $0721, 9               ; D4
    Note $0744, 9               ; F4
    dw 0

Tiles:                          ; tiles 0-3, 2bpp
    ds 16, 0                    ; 0: blank
    ds 16, $FF                  ; 1: solid (paddle / walls)
    db $3C,$3C, $7E,$7E, $FF,$FF, $FF,$FF, $FF,$FF, $FF,$FF, $7E,$7E, $3C,$3C  ; 2: ball
    db $00,$18, $00,$18, $00,$18, $00,$18, $00,$00, $00,$00, $00,$00, $00,$00  ; 3: centre dash
TilesEnd:

Font:                           ; 1bpp, tiles 4..27: 0-9, A E G I L N O P R S T U W Y
    db %00111100,%01100110,%01101110,%01110110,%01100110,%01100110,%00111100,0 ; 0
    db %00011000,%00111000,%00011000,%00011000,%00011000,%00011000,%01111110,0 ; 1
    db %00111100,%01100110,%00000110,%00001100,%00110000,%01100000,%01111110,0 ; 2
    db %00111100,%01100110,%00000110,%00011100,%00000110,%01100110,%00111100,0 ; 3
    db %00001100,%00011100,%00101100,%01001100,%01111110,%00001100,%00001100,0 ; 4
    db %01111110,%01100000,%01111100,%00000110,%00000110,%01100110,%00111100,0 ; 5
    db %00111100,%01100000,%01111100,%01100110,%01100110,%01100110,%00111100,0 ; 6
    db %01111110,%00000110,%00001100,%00011000,%00110000,%00110000,%00110000,0 ; 7
    db %00111100,%01100110,%01100110,%00111100,%01100110,%01100110,%00111100,0 ; 8
    db %00111100,%01100110,%01100110,%00111110,%00000110,%00001100,%00111000,0 ; 9
    db %00011000,%00111100,%01100110,%01100110,%01111110,%01100110,%01100110,0 ; A
    db %01111110,%01100000,%01100000,%01111000,%01100000,%01100000,%01111110,0 ; E
    db %00111100,%01100110,%01100000,%01101110,%01100110,%01100110,%00111110,0 ; G
    db %00111100,%00011000,%00011000,%00011000,%00011000,%00011000,%00111100,0 ; I
    db %01100000,%01100000,%01100000,%01100000,%01100000,%01100000,%01111110,0 ; L
    db %01100110,%01110110,%01111110,%01111110,%01101110,%01100110,%01100110,0 ; N
    db %00111100,%01100110,%01100110,%01100110,%01100110,%01100110,%00111100,0 ; O
    db %01111100,%01100110,%01100110,%01111100,%01100000,%01100000,%01100000,0 ; P
    db %01111100,%01100110,%01100110,%01111100,%01101100,%01100110,%01100110,0 ; R
    db %00111110,%01100000,%01100000,%00111100,%00000110,%00000110,%01111100,0 ; S
    db %01111110,%00011000,%00011000,%00011000,%00011000,%00011000,%00011000,0 ; T
    db %01100110,%01100110,%01100110,%01100110,%01100110,%01100110,%00111100,0 ; U
    db %11000110,%11000110,%11010110,%11010110,%11111110,%11101110,%11000110,0 ; W
    db %01100110,%01100110,%00111100,%00011000,%00011000,%00011000,%00011000,0 ; Y
FontEnd:
