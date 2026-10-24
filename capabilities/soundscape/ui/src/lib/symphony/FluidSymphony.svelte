<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import type { PresetKey, LayerKey, AudioLayers, AudioParams } from "./types";
  import { FluidSim } from "./fluidSim";
  import { PALETTES, PRESETS, SCAPES } from "./audioEngine";
  import { symphonyStore } from "./symphonyStore.svelte";
  import { formatSeed, parseSeed } from "./rng";

  type Props = {
    preset?: PresetKey;
    volume?: number;
    energy?: number;
    showControls?: boolean;
    showOverlay?: boolean;
    autoStart?: boolean;
    maxDpr?: number;
    simRes?: number;
    dyeRes?: number;
    /** Set when framed: collapsing back to the dock replaces leaving the page. */
    oncollapse?: () => void;
  };

  // The annotation form, not `$props<Props>()`: svelte-check 4.7.7 reports the generic call as
  // "Expected 0 type arguments", which collapses the prop types to `any`.
  let {
    preset = "edm" as PresetKey,
    volume = 0.65,
    energy = 0.7,
    showControls = true,
    showOverlay = true,
    autoStart = false,
    maxDpr = 1.25,
    simRes = 128,
    dyeRes = 640,
    oncollapse = undefined,
  }: Props = $props();

  let canvasRef = $state<HTMLCanvasElement | null>(null);
  let isSupported = $state(true);
  const isPlaying = $derived(symphonyStore.isPlaying);
  let overlayGone = $state(false);
  let advClosed = $state(true);
  let uiHidden = $state(false);

  const currentPreset = $derived(symphonyStore.preset);
  let pendingPreset = $state<PresetKey | null>(null);
  const currentVolume = $derived(symphonyStore.volume);
  const currentEnergy = $derived(symphonyStore.energy);
  const timerMinutes = $derived(
    symphonyStore.session?.scenario === "timer"
      ? Math.round(symphonyStore.session.duration_ms / 60_000)
      : null,
  );
  let adaptMode = $state(false);

  // The store holds the scape, because the conductor can change it from another
  // surface at any moment. This page renders that state instead of keeping a copy
  // it would have to keep in sync by hand.
  const layers = $derived(symphonyStore.layers);

  /** Where a muted layer comes back to, so the chip stays a toggle. */
  const lastLevel: Record<LayerKey, number> = {
    drums: 1,
    bass: 1,
    pads: 1,
    melody: 1,
    texture: 1,
  };

  const params = $derived(symphonyStore.params);

  let nowPlayingText = $state("");

  const fluidSim = new FluidSim();
  // The one engine, owned by the store and shared with the mini player. This page
  // used to construct a second one, so both could play at once into the same output.
  const audio = symphonyStore.getAudioEngine();

  let seedText = $state(formatSeed(audio.seed));
  let seedInput = $state<HTMLInputElement | null>(null);

  let animFrameId: number | null = null;
  let idleTimerId: ReturnType<typeof setTimeout> | null = null;
  let lastFrameTime = performance.now();
  let kickPulse = 0;
  let audioLevel = 0;
  let npAcc = 0;

  // Pointer state for fluid interaction
  let pointerDown = false;
  let pointerX = 0;
  let pointerY = 0;
  let pointerPx = 0;
  let pointerPy = 0;

  // Ambient wanderers
  const wanderers = [
    { fx: 0.11, fy: 0.17, px: 1.3, py: 0.4, ci: 0 },
    { fx: 0.07, fy: 0.13, px: 4.1, py: 2.2, ci: 1 },
    { fx: 0.15, fy: 0.09, px: 2.6, py: 5.0, ci: 2 },
  ];

  function daySegment() {
    const h = new Date().getHours();
    return h < 5 ? "night" : h < 9 ? "dawn" : h < 17 ? "day" : h < 22 ? "dusk" : "night";
  }

  const TINTS: Record<string, [number, number, number]> = {
    dawn: [1.12, 0.95, 0.85],
    day: [1.0, 1.0, 1.06],
    dusk: [1.08, 0.9, 1.12],
    night: [0.8, 0.85, 1.18],
  };

  function scaleColor(c: [number, number, number], k: number): [number, number, number] {
    if (audio.isScape()) {
      const tn = TINTS[daySegment()];
      return [c[0] * tn[0] * k, c[1] * tn[1] * k, c[2] * tn[2] * k];
    }
    return [c[0] * k, c[1] * k, c[2] * k];
  }

  function fluidSpeed() {
    const sc = SCAPES[currentPreset as keyof typeof SCAPES];
    return sc ? sc.fluid : 1;
  }

  function updateNowPlaying() {
    const pr = PRESETS[currentPreset];
    let txt = "♪ " + pr.label;
    if (audio.isScape()) {
      txt += " · " + Math.round(audio.bpm) + " BPM · " + (adaptMode ? "adaptive " : "") + daySegment();
      if (audio.arcPhase) txt += " · " + audio.arcPhase;
    }
    if (symphonyStore.session) txt += " · " + symphonyStore.sessionClock;
    if (pendingPreset) txt += " · → " + pendingPreset.toUpperCase() + " at the bar";
    nowPlayingText = txt;
  }

  function togglePlay() {
    void symphonyStore.togglePlay();
    updateNowPlaying();
  }

  function selectPreset(p: PresetKey) {
    // Landing is the engine's call — while playing it waits for the next bar, so
    // the UI follows the store rather than assuming the switch happened.
    symphonyStore.setPreset(p);
    pendingPreset = audio.pendingPreset;
    updateNowPlaying();
  }

  function updateParam(key: keyof AudioParams, val: number) {
    symphonyStore.setParam(key, val);
  }

  function setLayer(key: LayerKey, level: number) {
    if (level > 0) lastLevel[key] = level;
    symphonyStore.setLayer(key, level);
  }

  function toggleLayer(key: LayerKey) {
    setLayer(key, layers[key] > 0 ? 0 : lastLevel[key] || 1);
  }

  function rerollSeed() {
    seedText = formatSeed(symphonyStore.reseed());
  }

  function applySeed(e: Event) {
    const parsed = parseSeed((e.target as HTMLInputElement).value);
    if (parsed !== null) symphonyStore.setSeed(parsed);
    seedText = formatSeed(symphonyStore.seed);
  }

  function toggleAdapt() {
    adaptMode = !adaptMode;
    audio.adapt = adaptMode;
    updateNowPlaying();
  }

  function handleTimerChange(e: Event) {
    const val = (e.target as HTMLSelectElement).value;
    const m = parseInt(val, 10);
    symphonyStore.setTimer(m || null);
    updateNowPlaying();
  }

  function resetIdle() {
    if (uiHidden) return;
    uiHidden = false;
    if (idleTimerId) clearTimeout(idleTimerId);
    idleTimerId = setTimeout(() => {
      uiHidden = true;
    }, 5000);
  }

  // Pointer & Drag handling
  function getPointerPos(e: PointerEvent): [number, number] {
    if (!canvasRef) return [0, 0];
    const r = canvasRef.getBoundingClientRect();
    return [(e.clientX - r.left) / r.width, 1 - (e.clientY - r.top) / r.height];
  }

  function onPointerDown(e: PointerEvent) {
    pointerDown = true;
    [pointerX, pointerY] = getPointerPos(e);
    pointerPx = pointerX;
    pointerPy = pointerY;
    resetIdle();
  }

  function onPointerMove(e: PointerEvent) {
    resetIdle();
    if (!pointerDown) return;
    [pointerX, pointerY] = getPointerPos(e);
    const dx = (pointerX - pointerPx) * 5200;
    const dy = (pointerY - pointerPy) * 5200;
    const pal = PALETTES[currentPreset];
    const col = scaleColor(pal[Math.floor(Math.random() * pal.length)], 0.25);
    fluidSim.splat(pointerX, pointerY, dx, dy, col, 0.0028);
    pointerPx = pointerX;
    pointerPy = pointerY;
  }

  function onPointerUp() {
    pointerDown = false;
  }

  // Keybindings
  function onKeyDown(e: KeyboardEvent) {
    if (e.target && (e.target as HTMLElement).tagName === "INPUT") return;
    if (e.code === "Space") {
      e.preventDefault();
      togglePlay();
    } else if (e.key === "1") selectPreset("edm");
    else if (e.key === "2") selectPreset("ambient");
    else if (e.key === "3") selectPreset("lofi");
    else if (e.key === "4") selectPreset("focus");
    else if (e.key === "5") selectPreset("relax");
    else if (e.key === "6") selectPreset("sleep");
    else if (e.key === "m" || e.key === "M") {
      if (audio.master && audio.ctx) {
        const muted = audio.master.gain.value < 0.001;
        audio.master.gain.setTargetAtTime(
          muted ? currentVolume * currentVolume : 0,
          audio.ctx.currentTime,
          0.03
        );
      }
    } else if (e.key === "h" || e.key === "H") {
      uiHidden = !uiHidden;
    } else if (e.key === "f" || e.key === "F") {
      if (document.fullscreenElement) document.exitFullscreen();
      else document.documentElement.requestFullscreen();
    }
  }

  // Animation Frame Loop
  function frame(now: number) {
    const dt = Math.min((now - lastFrameTime) / 1000, 1 / 30);
    lastFrameTime = now;

    fluidSim.resize();

    // Sample audio frequency level
    if (audio.ctx && audio.playing && audio.analyser && audio.freqData) {
      audio.analyser.getByteFrequencyData(audio.freqData as never);
      let sum = 0;
      for (let i = 0; i < 24; i++) sum += audio.freqData[i];
      audioLevel = audioLevel * 0.8 + (sum / (24 * 255)) * 0.2;
    } else {
      audioLevel *= 0.95;
    }

    // Process music visual events
    audio.consumeMusicEvents((ev) => {
      const pal = PALETTES[currentPreset];
      const force = (0.5 + audio.energy) * fluidSpeed();
      if (ev.type === "kick") {
        kickPulse = Math.min(kickPulse + 0.28 * force, 0.9);
        const cx = 0.5 + (Math.random() - 0.5) * 0.25;
        const cy = 0.42 + (Math.random() - 0.5) * 0.2;
        const col = pal[Math.floor(Math.random() * pal.length)];
        for (let i = 0; i < 5; i++) {
          const a = (i / 5) * Math.PI * 2 + Math.random();
          fluidSim.splat(
            cx, cy,
            Math.cos(a) * 260 * force,
            Math.sin(a) * 260 * force,
            scaleColor(col, 0.16),
            0.0035
          );
        }
      } else if (ev.type === "note") {
        const x = 0.18 + (((ev.midi || 60) % 12) / 11) * 0.64 + (Math.random() - 0.5) * 0.06;
        const y = 0.5 + Math.random() * 0.3;
        const col = pal[(ev.midi || 60) % pal.length];
        fluidSim.splat(
          x, y,
          (Math.random() - 0.5) * 420 * force,
          (Math.random() - 0.3) * 380 * force,
          scaleColor(col, 0.3),
          0.0011
        );
      } else if (ev.type === "bass") {
        const x = 0.3 + Math.random() * 0.4;
        const col = pal[((ev.midi || 36) + 1) % pal.length];
        fluidSim.splat(
          x, 0.16,
          (Math.random() - 0.5) * 220 * force,
          240 * force,
          scaleColor(col, 0.18),
          0.0022
        );
      } else if (ev.type === "chord") {
        const col = pal[(ev.midi || 48) % pal.length];
        for (let i = 0; i < 3; i++) {
          fluidSim.splat(
            0.25 + Math.random() * 0.5,
            0.3 + Math.random() * 0.4,
            (Math.random() - 0.5) * 140,
            (Math.random() - 0.5) * 140,
            scaleColor(col, 0.1),
            0.005
          );
        }
      }
    });

    // Ambient fluid motion
    const pal = PALETTES[currentPreset];
    const fs = fluidSpeed();
    const amp = (0.18 + audioLevel * 1.4) * fs;
    const timeSec = now / 1000;
    for (const w of wanderers) {
      const x = 0.5 + 0.33 * Math.sin(timeSec * w.fx * Math.PI * 2 + w.px);
      const y = 0.5 + 0.3 * Math.sin(timeSec * w.fy * Math.PI * 2 + w.py);
      const dx = Math.cos(timeSec * w.fx * Math.PI * 2 + w.px) * w.fx * 40;
      const dy = Math.cos(timeSec * w.fy * Math.PI * 2 + w.py) * w.fy * 40;
      const col = scaleColor(pal[w.ci % pal.length], 0.005 + audioLevel * 0.02);
      fluidSim.splat(x, y, dx * amp * 60, dy * amp * 60, col, 0.0025);
    }

    // Step simulation and decay pulse
    fluidSim.stepSim(dt * fs);
    kickPulse *= Math.exp(-5.5 * dt);
    fluidSim.render(kickPulse);

    // Timer update
    npAcc += dt;
    if (npAcc > 1) {
      npAcc = 0;
      // Preset, volume and energy are read straight off the store; only what the
      // engine alone knows has to be pulled across.
      pendingPreset = audio.pendingPreset;
      // Not while it is being typed in — the field belongs to whoever has focus.
      if (document.activeElement !== seedInput) seedText = formatSeed(symphonyStore.seed);
      updateNowPlaying();
    }

    animFrameId = requestAnimationFrame(frame);
  }

  onMount(() => {
    // Mounting is not an edit. The conductor holds what should be playing, and a
    // page that pushed its own prop defaults on load would reset the scape for
    // every other surface every time someone opened it — which it used to do.
    overlayGone = symphonyStore.hasStarted || !showOverlay;
    seedText = formatSeed(symphonyStore.seed);

    if (!canvasRef) return;
    const ok = fluidSim.init(canvasRef, { simRes, dyeRes, maxDpr });
    if (!ok) {
      isSupported = false;
      return;
    }

    updateNowPlaying();

    if (autoStart && !symphonyStore.isPlaying) {
      overlayGone = true;
      togglePlay();
    }

    window.addEventListener("pointermove", resetIdle);
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("pointerup", onPointerUp);

    animFrameId = requestAnimationFrame(frame);
  });

  onDestroy(() => {
    if (animFrameId !== null) cancelAnimationFrame(animFrameId);
    if (idleTimerId !== null) clearTimeout(idleTimerId);

    window.removeEventListener("pointermove", resetIdle);
    window.removeEventListener("keydown", onKeyDown);
    window.removeEventListener("pointerup", onPointerUp);

    // The engine outlives this page on purpose — leaving /symphony must not stop
    // what the mini player is still showing. The store owns its lifecycle.
    fluidSim.destroy();
  });
</script>

<div class="symphony-container">
  <canvas
    bind:this={canvasRef}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
  ></canvas>

  {#if showControls}
    <!-- Framed, leaving is collapsing back to the dock; standalone it is a link home. -->
    {#if oncollapse}
      <button class="back-btn" class:hidden-ui={uiHidden} onclick={oncollapse} title="Back to bar">
        ↓ Bar
      </button>
    {:else}
      <a href="/" class="back-btn" class:hidden-ui={uiHidden} title="Back to the Sjel dashboard">
        ← Sjel Dashboard
      </a>
    {/if}

    <!-- Titlebar -->
    <div class="titlebar" class:hidden-ui={uiHidden}>
      <h1>FLUID SYMPHONY</h1>
      <p>the fluid dances to the music — drag to join in</p>
      <div class="nowplaying">{nowPlayingText}</div>
    </div>

    <!-- Advanced Panel -->
    <div class="adv" class:closed={advClosed} class:hidden-ui={uiHidden}>
      <div class="advrow">
        {#each Object.keys(params) as key}
          <div class="sliderbox">
            <label for="param-{key}">{key}</label>
            <input
              id="param-{key}"
              type="range"
              min="0"
              max="100"
              value={params[key as keyof AudioParams] * 100}
              oninput={(e) => updateParam(key as keyof AudioParams, +(e.target as HTMLInputElement).value / 100)}
            />
          </div>
        {/each}
      </div>

      <div class="advrow">
        <span class="advlabel">LAYERS</span>
        {#each Object.keys(layers) as layerKey}
          <button
            class="chip layer"
            class:active={layers[layerKey as LayerKey] > 0}
            onclick={() => toggleLayer(layerKey as LayerKey)}
          >
            {layerKey.toUpperCase()}
          </button>
        {/each}

        <span class="advlabel">·</span>
        <button
          class="chip"
          class:active={adaptMode}
          onclick={toggleAdapt}
          title="adapts to time of day + your activity"
        >
          ADAPTIVE
        </button>

        <select id="timer" value={timerMinutes ?? ""} onchange={handleTimerChange}>
          <option value="">
            {symphonyStore.session
              ? `${symphonyStore.sessionLabel?.toUpperCase()} · ${symphonyStore.sessionClock}`
              : "TIMER OFF"}
          </option>
          <option value="15">15 MIN</option>
          <option value="25">25 MIN</option>
          <option value="45">45 MIN</option>
        </select>

        <span class="advlabel">SEED</span>
        <input
          id="seed"
          class="seedinput"
          bind:this={seedInput}
          value={seedText}
          onchange={applySeed}
          title="this session's material — the same seed plays the same notes"
        />
        <button class="chip" onclick={rerollSeed} title="new material, same settings">↻</button>
      </div>
      <div class="advrow">
        <span class="advlabel">MIX</span>
        {#each Object.keys(layers) as layerKey}
          <div class="sliderbox">
            <label for="layer-{layerKey}">{layerKey}</label>
            <input
              id="layer-{layerKey}"
              type="range"
              min="0"
              max="100"
              value={layers[layerKey as LayerKey] * 100}
              oninput={(e) => setLayer(layerKey as LayerKey, +(e.target as HTMLInputElement).value / 100)}
            />
          </div>
        {/each}
      </div>

      <div class="advhint">
        SCENE SLIDERS SHAPE FOCUS · RELAX · SLEEP — MIX APPLIES FROM THE NEXT NOTE
      </div>
    </div>

    <!-- Bottom Controls Bar -->
    <div class="controls" class:hidden-ui={uiHidden}>
      <button class="playbtn" onclick={togglePlay} title="play / pause (space)">
        {isPlaying ? "❚❚" : "▶"}
      </button>

      <button
        class="pill"
        class:active={currentPreset === "edm"}
        class:pending={pendingPreset === "edm"}
        onclick={() => selectPreset("edm")}
      >
        CLASSICAL × EDM
      </button>
      <button
        class="pill"
        class:active={currentPreset === "ambient"}
        class:pending={pendingPreset === "ambient"}
        onclick={() => selectPreset("ambient")}
      >
        EPIC AMBIENT
      </button>
      <button
        class="pill"
        class:active={currentPreset === "lofi"}
        class:pending={pendingPreset === "lofi"}
        onclick={() => selectPreset("lofi")}
      >
        LO-FI
      </button>

      <div class="vdiv"></div>

      <button
        class="pill scene"
        class:active={currentPreset === "focus"}
        class:pending={pendingPreset === "focus"}
        onclick={() => selectPreset("focus")}
      >
        FOCUS
      </button>
      <button
        class="pill scene"
        class:active={currentPreset === "relax"}
        class:pending={pendingPreset === "relax"}
        onclick={() => selectPreset("relax")}
      >
        RELAX
      </button>
      <button
        class="pill scene"
        class:active={currentPreset === "sleep"}
        class:pending={pendingPreset === "sleep"}
        onclick={() => selectPreset("sleep")}
      >
        SLEEP
      </button>

      <div class="sliderbox">
        <label for="vol-slider">volume</label>
        <input
          id="vol-slider"
          type="range"
          min="0"
          max="100"
          value={currentVolume * 100}
          oninput={(e) => symphonyStore.setVolume(+(e.target as HTMLInputElement).value / 100)}
        />
      </div>

      <div class="sliderbox">
        <label for="nrg-slider">intensity</label>
        <input
          id="nrg-slider"
          type="range"
          min="0"
          max="100"
          value={currentEnergy * 100}
          oninput={(e) => symphonyStore.setEnergy(+(e.target as HTMLInputElement).value / 100)}
        />
      </div>

      <button
        class="pill"
        onclick={() => (advClosed = !advClosed)}
        title="advanced controls"
      >
        ⚙
      </button>
    </div>
  {/if}

  {#if showOverlay && !overlayGone}
    <div class="overlay" class:gone={overlayGone}>
      <h1>FLUID SYMPHONY</h1>
      <p>
        Generative music composed in real time by code, never sampled — with a fluid simulation that moves to every beat.<br />
        3 performances · 3 adaptive scenes (focus / relax / sleep)<br />
        1–6 switch styles · space play/pause · M mute · H hide UI · F fullscreen
      </p>
      {#if isSupported}
        <button
          class="startbtn"
          onclick={() => {
            overlayGone = true;
            if (!isPlaying) togglePlay();
          }}
        >
          BEGIN
        </button>
      {:else}
        <div class="err">
          WebGL2 with float textures is required — try Chrome, Firefox or Edge.
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .symphony-container {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 100vh;
    overflow: hidden;
    background: #05030a;
    font-family: -apple-system, 'Helvetica Neue', Helvetica, Arial, sans-serif;
    color: #e8e4f0;
    user-select: none;
  }

  .back-btn {
    position: absolute;
    top: 24px;
    left: 24px;
    z-index: 10;
    padding: 8px 16px;
    border-radius: 999px;
    font-size: 11px;
    letter-spacing: 0.12em;
    border: 1px solid rgba(160, 120, 255, 0.25);
    background: rgba(12, 8, 22, 0.65);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    color: rgba(232, 228, 240, 0.85);
    text-decoration: none;
    transition: all 0.25s ease;
    /* The same control is a link standalone and a button when framed. */
    font-family: inherit;
    cursor: pointer;
  }
  .back-btn:hover {
    border-color: rgba(190, 150, 255, 0.8);
    background: rgba(130, 70, 255, 0.35);
    color: #ffffff;
    box-shadow: 0 0 16px rgba(140, 80, 255, 0.35);
  }

  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
    cursor: crosshair;
  }

  .titlebar {
    position: absolute;
    top: 26px;
    left: 0;
    right: 0;
    text-align: center;
    pointer-events: none;
    transition: opacity 0.8s ease;
    z-index: 5;
  }
  .titlebar h1 {
    font-size: 26px;
    font-weight: 300;
    letter-spacing: 0.55em;
    text-indent: 0.55em;
    color: #f2eefb;
    text-shadow: 0 0 24px rgba(160, 90, 255, 0.35);
    margin: 0;
  }
  .titlebar p {
    margin-top: 8px;
    font-size: 10.5px;
    letter-spacing: 0.28em;
    text-indent: 0.28em;
    color: rgba(232, 228, 240, 0.55);
  }
  .nowplaying {
    margin-top: 6px;
    font-size: 10px;
    letter-spacing: 0.22em;
    text-indent: 0.22em;
    color: rgba(190, 150, 255, 0.75);
  }

  .controls {
    position: absolute;
    bottom: 24px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 20px;
    border-radius: 16px;
    background: rgba(12, 8, 22, 0.65);
    backdrop-filter: blur(14px);
    -webkit-backdrop-filter: blur(14px);
    border: 1px solid rgba(160, 120, 255, 0.16);
    transition: opacity 0.8s ease;
    z-index: 5;
  }
  .hidden-ui {
    opacity: 0 !important;
    pointer-events: none !important;
  }
  .vdiv {
    width: 1px;
    height: 26px;
    background: rgba(160, 120, 255, 0.25);
    flex: none;
  }

  .pill {
    padding: 7px 13px;
    border-radius: 999px;
    font-size: 11px;
    letter-spacing: 0.12em;
    border: 1px solid rgba(160, 120, 255, 0.25);
    background: transparent;
    color: rgba(232, 228, 240, 0.7);
    cursor: pointer;
    transition: all 0.25s;
    white-space: nowrap;
  }
  .pill:hover {
    border-color: rgba(190, 150, 255, 0.6);
    color: #fff;
  }
  /* Queued for the next bar — click it again to switch now. */
  .pill.pending {
    border-style: dashed;
    border-color: rgba(190, 150, 255, 0.75);
    color: #fff;
    animation: waiting 1.6s ease-in-out infinite;
  }
  @keyframes waiting {
    0%,
    100% {
      box-shadow: 0 0 0 rgba(140, 80, 255, 0);
    }
    50% {
      box-shadow: 0 0 14px rgba(140, 80, 255, 0.45);
    }
  }
  .pill.active {
    background: rgba(130, 70, 255, 0.28);
    border-color: rgba(190, 150, 255, 0.85);
    color: #fff;
    box-shadow: 0 0 16px rgba(140, 80, 255, 0.35);
  }
  .pill.scene.active {
    background: rgba(40, 140, 200, 0.3);
    border-color: rgba(110, 200, 255, 0.85);
    box-shadow: 0 0 16px rgba(80, 170, 255, 0.35);
  }

  .playbtn {
    width: 42px;
    height: 42px;
    border-radius: 50%;
    border: 1px solid rgba(190, 150, 255, 0.5);
    background: rgba(130, 70, 255, 0.25);
    color: #fff;
    font-size: 15px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.25s;
    flex: none;
  }
  .playbtn:hover {
    background: rgba(150, 90, 255, 0.45);
    box-shadow: 0 0 18px rgba(150, 90, 255, 0.5);
  }

  .sliderbox {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .sliderbox label {
    font-size: 9px;
    letter-spacing: 0.18em;
    color: rgba(232, 228, 240, 0.5);
    text-transform: uppercase;
  }
  input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    width: 100px;
    height: 3px;
    border-radius: 2px;
    background: rgba(160, 120, 255, 0.25);
    outline: none;
    cursor: pointer;
  }
  input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: #b893ff;
    box-shadow: 0 0 10px rgba(160, 100, 255, 0.8);
  }

  .adv {
    position: absolute;
    bottom: 104px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 22px;
    border-radius: 16px;
    background: rgba(12, 8, 22, 0.7);
    backdrop-filter: blur(14px);
    -webkit-backdrop-filter: blur(14px);
    border: 1px solid rgba(160, 120, 255, 0.16);
    transition: opacity 0.8s ease;
    z-index: 6;
  }
  .adv.closed {
    display: none;
  }
  .advrow {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-wrap: wrap;
    justify-content: center;
  }
  .advlabel {
    font-size: 9px;
    letter-spacing: 0.2em;
    color: rgba(232, 228, 240, 0.45);
  }
  .seedinput {
    width: 78px;
    padding: 5px 9px;
    border-radius: 999px;
    font-family: inherit;
    font-size: 9.5px;
    letter-spacing: 0.14em;
    text-align: center;
    border: 1px solid rgba(160, 120, 255, 0.22);
    background: transparent;
    color: rgba(232, 228, 240, 0.7);
    outline: none;
  }
  .seedinput:focus {
    border-color: rgba(190, 150, 255, 0.55);
  }
  .chip {
    padding: 5px 11px;
    border-radius: 999px;
    font-size: 9.5px;
    letter-spacing: 0.14em;
    border: 1px solid rgba(160, 120, 255, 0.22);
    background: transparent;
    color: rgba(232, 228, 240, 0.45);
    cursor: pointer;
    transition: all 0.2s;
  }
  .chip:hover {
    border-color: rgba(190, 150, 255, 0.55);
    color: #fff;
  }
  .chip.active {
    background: rgba(130, 70, 255, 0.22);
    border-color: rgba(190, 150, 255, 0.7);
    color: #fff;
  }
  #timer {
    background: rgba(20, 12, 38, 0.8);
    color: rgba(232, 228, 240, 0.75);
    border: 1px solid rgba(160, 120, 255, 0.25);
    border-radius: 999px;
    padding: 5px 10px;
    font-size: 9.5px;
    letter-spacing: 0.12em;
    outline: none;
    cursor: pointer;
  }
  .advhint {
    font-size: 8.5px;
    letter-spacing: 0.16em;
    color: rgba(232, 228, 240, 0.3);
    text-align: center;
  }

  .overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 22px;
    background: radial-gradient(ellipse at center, #0d0718 0%, #05030a 70%);
    z-index: 20;
    transition: opacity 1.2s ease;
  }
  .overlay.gone {
    opacity: 0;
    pointer-events: none;
  }
  .overlay h1 {
    font-size: 30px;
    font-weight: 200;
    letter-spacing: 0.6em;
    text-indent: 0.6em;
    margin: 0;
  }
  .overlay p {
    font-size: 12px;
    letter-spacing: 0.25em;
    text-indent: 0.25em;
    color: rgba(232, 228, 240, 0.5);
    max-width: 600px;
    text-align: center;
    line-height: 2;
  }
  .startbtn {
    margin-top: 10px;
    padding: 14px 44px;
    border-radius: 999px;
    font-size: 13px;
    letter-spacing: 0.3em;
    text-indent: 0.3em;
    border: 1px solid rgba(190, 150, 255, 0.5);
    background: rgba(130, 70, 255, 0.18);
    color: #fff;
    cursor: pointer;
    transition: all 0.3s;
  }
  .startbtn:hover {
    background: rgba(150, 90, 255, 0.4);
    box-shadow: 0 0 30px rgba(150, 90, 255, 0.5);
    transform: scale(1.04);
  }
  .err {
    color: #ff7a7a;
    font-size: 13px;
    letter-spacing: 0.1em;
  }
</style>
