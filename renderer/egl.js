/* ============================================================================
   VisualLLM — the field. Esoteric Generative Luxury, rendered.

   This file is the GPU half of the piece. Two fragment programs:

     SIM   a Gray–Scott reaction–diffusion system run on a ping-pong pair of
           float textures. Two chemicals, U and V, diffuse and react: V feeds
           on U, patterns accrete — coral, leopard, labyrinth — the same
           mathematics as animal markings. The feed/kill constants below are
           tuned to the labyrinth regime. Lanes with live requests seed V into
           the field at their own screen position, so traffic does not animate
           a chart; it disturbs a chemical system and the whole interface
           ripples with heavy, physical inertia.

     PAINT the compositor. Reads the simulation, the Piet macro-structure,
           and a woven glyph micro-texture, and produces the final field:
           brushed obsidian, volumetric acrylic slabs at three depths with
           cheap refraction (the sim bleeds through, offset), a travelling
           hash shimmer, and a palette whose six hues are the Piet hues.

   Palette discipline: the Piet interpreter lives in app.js and executes a
   program painted in the very hues below. Its terminal stack top sets the
   feed rate of this simulation. Code as canvas; canvas as code; the loop
   closed through the execution.

   Everything degrades honestly: no float textures → no sim, a static woven
   field; no WebGL → the DOM stands alone on flat obsidian.
   ========================================================================== */

'use strict'

/* ============================================================================
   The CPU fallback. The same chemistry, the same grade, no GPU.
   A coarse Gray–Scott grid (SIM_W×SIM_H) is integrated in typed arrays and
   blitted to an offscreen canvas; the visible canvas draws that field
   magnified (bilinear-soft) beneath the same seam, vignette, and shimmer
   passes the shader would have run.
   ========================================================================== */
function makeCpuFallback(canvas) {
  const ctx2d = canvas.getContext('2d')
  if (!ctx2d) return { mount: () => {}, ok: false }

  const SIM_W = 132, SIM_H = 84
  const N = SIM_W * SIM_H
  let U = new Float32Array(N), V = new Float32Array(N)
  let U2 = new Float32Array(N), V2 = new Float32Array(N)
  const off = document.createElement('canvas')
  off.width = SIM_W; off.height = SIM_H
  const offCtx = off.getContext('2d')
  const img = offCtx.createImageData(SIM_W, SIM_H)

  const state = {
    feed: 0.0545, kill: 0.0620,
    seed: [0.5, 0.5], seedAmt: 0,
    hashRate: 0, cursor: [0.5, 0.5],
    running: false, mounted: false,
  }
  const reduced = matchMedia('(prefers-reduced-motion: reduce)')

  function seedCulture() {
    U.fill(1); V.fill(0)
    for (let i = 0; i < N; i++) if (Math.random() < 0.016) V[i] = 0.9
  }
  seedCulture()

  function stepSim() {
    const f = state.feed, k = state.kill
    for (let y = 0; y < SIM_H; y++) {
      const ym = (y - 1 + SIM_H) % SIM_H, yp = (y + 1) % SIM_H
      for (let x = 0; x < SIM_W; x++) {
        const xm = (x - 1 + SIM_W) % SIM_W, xp = (x + 1) % SIM_W
        const i = y * SIM_W + x
        const u = U[i], v = V[i]
        const lapU =
          0.2 * (U[y * SIM_W + xp] + U[y * SIM_W + xm] + U[yp * SIM_W + x] + U[ym * SIM_W + x]) +
          0.05 * (U[yp * SIM_W + xp] + U[yp * SIM_W + xm] + U[ym * SIM_W + xp] + U[ym * SIM_W + xm]) - u
        const lapV =
          0.2 * (V[y * SIM_W + xp] + V[y * SIM_W + xm] + V[yp * SIM_W + x] + V[ym * SIM_W + x]) +
          0.05 * (V[yp * SIM_W + xp] + V[yp * SIM_W + xm] + V[ym * SIM_W + xp] + V[ym * SIM_W + xm]) - v
        const uvv = u * v * v
        U2[i] = u + (0.2097 * lapU - uvv + f * (1 - u))
        V2[i] = v + (0.105 * lapV + uvv - (f + k) * v)
      }
    }
    // the syringe
    if (state.seedAmt > 0.003) {
      const cx = state.seed[0] * SIM_W, cy = state.seed[1] * SIM_H
      const r = Math.max(2, SIM_W * 0.045)
      for (let y = 0; y < SIM_H; y++) for (let x = 0; x < SIM_W; x++) {
        const d = Math.hypot(x - cx, y - cy) / r
        if (d < 1) V2[y * SIM_W + x] += state.seedAmt * (1 - d)
      }
      state.seedAmt *= 0.86
    }
    ;[U, U2] = [U2, U]; [V, V2] = [V2, V]
  }

  function paint(t) {
    // chemistry → pixels, through the obsidian grade. The reaction's own
    // scale is amplified so the labyrinth reads across a room, not just at
    // a microscope: V above the floor blooms coral; the 1−U shadow carries
    // the blue of unspent nutrient.
    const d = img.data
    for (let i = 0; i < N; i++) {
      const v = Math.min(1, Math.max(0, V[i]))
      const spent = 1 - U[i]
      const vein = Math.min(1, Math.max(0, v - 0.05) * 3.4)
      // obsidian warmed by the reaction's embers: a faint violet-grey from
      // the unspent nutrient, blooming coral where V concentrates.
      let r = 10 + 30 * spent + 205 * vein
      let g = 11 + 22 * spent + 74 * vein
      let b = 17 + 34 * spent + 96 * vein
      d[i * 4] = r; d[i * 4 + 1] = g; d[i * 4 + 2] = b; d[i * 4 + 3] = 255
    }
    offCtx.putImageData(img, 0, 0)

    const w = canvas.width, h = canvas.height
    ctx2d.imageSmoothingEnabled = true
    ctx2d.imageSmoothingQuality = 'high'
    ctx2d.clearRect(0, 0, w, h)
    ctx2d.drawImage(off, 0, 0, w, h)

    /* t3 — the weave: microscopic glyph-rows riding the chemistry. Rows are
       a hairline apart; within a row, the glyph cells brighten where the
       field's V is high — an ancient, high-tech text the reaction is
       spelling, too small to read and too present to miss. */
    let meanV = 0
    for (let i = 0; i < N; i += 7) meanV += V[i]
    meanV /= (N / 7)
    const rowH = Math.max(3, Math.round(h / 220))
    ctx2d.fillStyle = `rgba(150,160,185,${0.05 + meanV * 0.06})`
    for (let y = 0; y < h; y += rowH) {
      // sample the field along this row's centre
      const simY = Math.floor((y / h) * SIM_H)
      for (let x = 0; x < w; x += rowH * 2) {
        const simX = Math.floor((x / w) * SIM_W)
        const v = V[simY * SIM_W + simX] || 0
        // a pseudo-glyph: present or absent by a hash of position and row
        const g = (Math.sin(x * 12.9898 + y * 78.233) * 43758.5453) % 1
        if (g > 0.55 && v > 0.05) {
          ctx2d.globalAlpha = Math.min(0.16, 0.03 + v * 0.18)
          ctx2d.fillRect(x, y, rowH * 1.2, 1)
        }
      }
    }
    ctx2d.globalAlpha = 1

    // seams — the piet macro-structure's joints, glowing with the mean V
    ctx2d.strokeStyle = `rgba(120,130,155,${0.10 + meanV * 0.25})`
    ctx2d.lineWidth = 1
    const cuts = [0.272, 0.618]
    for (const c of cuts) {
      ctx2d.beginPath(); ctx2d.moveTo(w * c, 0); ctx2d.lineTo(w * c, h); ctx2d.stroke()
    }
    ctx2d.beginPath(); ctx2d.moveTo(w * 0.272, h * 0.34); ctx2d.lineTo(w, h * 0.34); ctx2d.stroke()

    // the shimmer — one travelling band of light, rate from the event clock
    if (!reduced.matches) {
      const sweep = (t * (0.02 + state.hashRate * 0.05)) % 1
      const bx = sweep * (w + h * 0.35) - h * 0.35
      const grad = ctx2d.createLinearGradient(bx - 60, 0, bx + 60, h * 0.35)
      grad.addColorStop(0, 'rgba(90,95,115,0)')
      grad.addColorStop(0.5, `rgba(150,155,180,${0.05 + meanV * 0.05})`)
      grad.addColorStop(1, 'rgba(90,95,115,0)')
      ctx2d.fillStyle = grad
      ctx2d.fillRect(0, 0, w, h)
    }

    // vignette
    const vig = ctx2d.createRadialGradient(w / 2, h / 2, h * 0.34, w / 2, h / 2, h * 0.95)
    vig.addColorStop(0, 'rgba(0,0,0,0)')
    vig.addColorStop(1, 'rgba(2,3,5,0.55)')
    ctx2d.fillStyle = vig
    ctx2d.fillRect(0, 0, w, h)
  }

  function resize() {
    const dpr = Math.min(devicePixelRatio || 1, 1.5)
    canvas.width = Math.round(innerWidth * dpr)
    canvas.height = Math.round(innerHeight * dpr)
  }
  addEventListener('resize', resize)

  let acc = 0, last = 0
  function frame(now) {
    if (!state.running) return
    requestAnimationFrame(frame)
    const dt = now - last
    last = now
    acc += dt
    // sim at ~24 Hz, paint every frame
    if (acc > 42 && !reduced.matches) {
      acc = 0
      for (let i = 0; i < 5; i++) stepSim()
    }
    paint(now / 1000)
  }

  return {
    ok: true,
    cpu: true,
    mount(parent) {
      if (state.mounted) return
      state.mounted = true
      parent.prepend(canvas)
      resize()
      state.running = true
      requestAnimationFrame(frame)
    },
    disturb(x, y, amt = 0.8) {
      state.seed = [x, 1 - y]
      state.seedAmt = Math.min(1, state.seedAmt + amt)
    },
    setFeed(f) { state.feed = 0.037 + 0.03 * Math.min(1, Math.max(0, f)) },
    setHashRate(r) { state.hashRate = Math.min(1, r) },
    setCursor(x, y) { state.cursor = [x, 1 - y] },
  }
}

const EGL = (() => {
  const canvas = document.createElement('canvas')
  canvas.className = 'egl-field'
  canvas.setAttribute('aria-hidden', 'true')

  const gl = canvas.getContext('webgl', {
    alpha: false,
    antialias: false,
    depth: false,
    stencil: false,
    powerPreference: 'low-power',
    preserveDrawingBuffer: false,
  })

  /* When the GPU is refused — a headless preview, a locked-down webview —
     the chemistry does not stop; it changes hands. The same Gray–Scott
     system is integrated on the CPU at low resolution and painted through a
     2D context with the same obsidian grade, the same seams, the same
     shimmer. Heavier per frame, so the fallback runs a coarser field at a
     slower clock — the building still breathes, just in a lower light. */
  if (!gl) return makeCpuFallback(canvas)

  /* ------------------------------------------------------------- shaders */

  const QUAD_VS = `
attribute vec2 aP;
varying vec2 vUv;
void main() { vUv = aP * 0.5 + 0.5; gl_Position = vec4(aP, 0.0, 1.0); }
`

  // Gray–Scott. rg = (U, V). One texel ≈ one cell; nine-point Laplacian.
  const SIM_FS = `
precision highp float;
varying vec2 vUv;
uniform sampler2D uPrev;
uniform vec2  uTexel;
uniform float uFeed;      // set by the executed Piet program (see app.js)
uniform float uKill;
uniform vec2  uSeed;      // where live traffic is, in field space
uniform float uSeedAmt;   // how strongly it disturbs the chemistry
uniform float uSeedRad;
uniform float uTime;

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }

void main() {
  vec2 c = texture2D(uPrev, vUv).rg;
  vec2 lap = -c;
  lap += 0.20 * texture2D(uPrev, vUv + vec2( uTexel.x, 0.0)).rg;
  lap += 0.20 * texture2D(uPrev, vUv + vec2(-uTexel.x, 0.0)).rg;
  lap += 0.20 * texture2D(uPrev, vUv + vec2(0.0,  uTexel.y)).rg;
  lap += 0.20 * texture2D(uPrev, vUv + vec2(0.0, -uTexel.y)).rg;
  lap += 0.05 * texture2D(uPrev, vUv + uTexel).rg;
  lap += 0.05 * texture2D(uPrev, vUv - uTexel).rg;
  lap += 0.05 * texture2D(uPrev, vUv + vec2( uTexel.x, -uTexel.y)).rg;
  lap += 0.05 * texture2D(uPrev, vUv + vec2(-uTexel.x,  uTexel.y)).rg;

  float u = c.r, v = c.g;
  float uvv = u * v * v;
  // Diffusion rates chosen so the pattern's wavelength reads as *fabric*
  // at panel scale — roughly a thumb's width between features.
  float du = 0.2097 * lap.x - uvv + uFeed * (1.0 - u);
  float dv = 0.105  * lap.y + uvv - (uFeed + uKill) * v;
  u += du; v += dv;

  // Traffic: a soft syringe of V. Not a sprite — a chemical disturbance
  // that the system then carries, spreads, and forgets on its own time.
  float d = distance(vUv * vec2(1.78, 1.0), uSeed * vec2(1.78, 1.0));
  v += uSeedAmt * smoothstep(uSeedRad, 0.0, d);

  gl_FragColor = vec4(clamp(u, 0.0, 1.0), clamp(v, 0.0, 1.0), 0.0, 1.0);
}
`

  /* The compositor. Everything the eye sees that is not DOM:
       t0  obsidian ground, subtly brushed, breathing with the sim's mean V
       t1  the Piet macro-structure — an asymmetric mondrian grid whose seams
           glow where chemistry concentrates beneath them
       t2  volumetric slabs at three parallax depths; the field refracts
           through them (uv offset by slab normal)
       t3  the weave — microscopic glyph-ish microstructure riding the sim
       t4  cryptographic shimmer: a hash raymarch sweeping one thin band of
           light across the field; position driven by uHashRate
       t5  vignette and grade */
  const PAINT_FS = `
precision highp float;
varying vec2 vUv;
uniform sampler2D uSim;
uniform vec2  uRes;
uniform float uTime;
uniform float uCursor;     // cursor.x in field space — the field leans toward it
uniform vec2  uCursorV;
uniform float uHashRate;   // events/s — the shimmer's sweep rate
uniform float uFeed;

#define PIET_RED    vec3(0.749, 0.000, 0.000)
#define PIET_YELLOW vec3(0.749, 0.749, 0.000)
#define PIET_GREEN  vec3(0.000, 0.749, 0.000)
#define PIET_CYAN   vec3(0.000, 0.749, 0.749)
#define PIET_BLUE   vec3(0.000, 0.000, 0.749)
#define PIET_MAGENTA vec3(0.749, 0.000, 0.749)

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float hash1(float n) { return fract(sin(n) * 43758.5453); }
float vnoise(vec2 p) {
  vec2 i = floor(p), f = fract(p);
  f = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash(i), hash(i + vec2(1, 0)), f.x),
             mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), f.x), f.y);
}
float fbm(vec2 p) {
  float a = 0.5, r = 0.0;
  for (int i = 0; i < 4; i++) { r += a * vnoise(p); p *= 2.03; a *= 0.5; }
  return r;
}

/* The macro-structure: one asymmetric recursive subdivision of the window,
   deterministic, cut along the golden angle's cousin (0.618…). Returns
   (cellId, distToSeam). This is the same division the DOM slabs align to —
   read the constants in egl.js:PIET_SLABS. */
vec2 pietCell(vec2 uv) {
  float id = 0.0;
  vec2 lo = vec2(0.0), hi = vec2(1.0);
  float seam = 1e3;
  for (int i = 0; i < 3; i++) {
    vec2 span = hi - lo;
    float cut = 0.382 + 0.236 * hash1(id + float(i) * 17.0);
    bool vert = span.x > span.y;
    float s = vert ? span.x * cut : span.y * cut;
    if (vert) {
      float x = lo.x + s;
      seam = min(seam, abs(uv.x - x) / span.x);
      if (uv.x < x) { hi.x = x; id = id * 2.0 + 1.0; }
      else          { lo.x = x; id = id * 2.0 + 2.0; }
    } else {
      float y = lo.y + s;
      seam = min(seam, abs(uv.y - y) / span.y);
      if (uv.y < y) { hi.y = y; id = id * 2.0 + 1.0; }
      else          { lo.y = y; id = id * 2.0 + 2.0; }
    }
  }
  return vec2(id, seam);
}

vec3 pietHue(float id) {
  float h = mod(id * 2.399963, 6.0); // golden angle over the hue ring
  if (h < 1.0) return PIET_RED;
  if (h < 2.0) return PIET_YELLOW;
  if (h < 3.0) return PIET_GREEN;
  if (h < 4.0) return PIET_CYAN;
  if (h < 5.0) return PIET_BLUE;
  return PIET_MAGENTA;
}

void main() {
  vec2 uv = vUv;
  vec2 asp = vec2(uRes.x / uRes.y, 1.0);

  /* --- the chemistry, sampled thrice: once flat, once refracted through
     the near slab, once through the far. Refraction here is an offset of
     the sample point by the slab's pseudo-normal — the pattern *bends*
     through the acrylic. */
  float vFlat = texture2D(uSim, uv).g;

  vec2 cell = pietCell(uv);
  float seam = cell.y;

  // Pseudo-normal from the seam: strongest bending at cell edges.
  vec2 nrm = vec2(dFdx(vFlat), dFdy(vFlat)) * 24.0;
  vec2 r1 = uv + nrm * 0.010;   // near slab
  vec2 r2 = uv + nrm * 0.026;   // far installation
  float vNear = texture2D(uSim, r1).g;
  float vFar  = texture2D(uSim, r2).g;

  /* t0 — obsidian ground. Not black: a deep blue-grey with a faint brushed
     anisotropy and the sim's far field breathing underneath. */
  float brush = fbm(vec2(uv.x * 3.0, uv.y * 260.0)) * 0.5
              + fbm(vec2(uv.x * 7.0, uv.y * 90.0)) * 0.5;
  vec3 col = vec3(0.028, 0.031, 0.043);
  col += vec3(0.012, 0.013, 0.020) * brush;
  col += vec3(0.020, 0.010, 0.012) * vFar;

  /* t1 — the seams: hairline joints in the macro-structure, glowing where
     chemistry presses against them. The building's grout is alive. */
  float seamGlow = smoothstep(0.012, 0.0, seam);
  col += vec3(0.10, 0.11, 0.14) * seamGlow;
  col += vec3(0.55, 0.14, 0.13) * seamGlow * smoothstep(0.25, 0.7, vNear);

  /* t2 — the slabs: three depths of acrylic. Each slab tints what is behind
     it by its Piet hue, attenuated by depth, and throws a thin specular
     along its lit seam. */
  vec3 hue = pietHue(cell.x);
  float cellV = smoothstep(0.35, 0.9, vNear);
  // near tint — restrained; luxury is discipline
  col = mix(col, hue * 0.16 + col * 0.84, 0.10 + 0.20 * cellV * hash1(cell.x));
  // specular kiss on seams facing the (fixed, top-left) virtual sun
  float spec = smoothstep(0.004, 0.0, seam) * (0.35 + 0.65 * cellV);
  col += vec3(0.9, 0.92, 1.0) * spec * 0.10;

  /* t3 — the weave: microscopic glyph-rows riding the chemistry. At native
     resolution these sit at the edge of legibility — an ancient high-tech
     text woven into the material. Detail rules: they brighten with V, never
     with time, so the weave is topography, not animation. */
  float row = floor(uv.y * uRes.y / 3.0);
  float glyph = step(0.72, hash(vec2(row, floor(uv.x * uRes.x / 3.0)) + floor(row * 0.13)));
  float weave = glyph * smoothstep(0.18, 0.55, vFlat);
  col += vec3(0.10, 0.115, 0.14) * weave * 0.5;

  /* t4 — the shimmer: one thin band of light sweeping the field. Its
     position is a hash of the event clock — background allocation made
     visible as a travelling sheen. Slow is expensive; fast is cheap. */
  float sweep = fract(uTime * (0.02 + uHashRate * 0.05));
  float band = smoothstep(0.06, 0.0, abs(uv.x * asp.x + uv.y * 0.35 - sweep * (asp.x + 0.7)));
  col += vec3(0.30, 0.32, 0.38) * band * (0.25 + 0.5 * vNear);

  /* cursor gravity — the field leans toward the hand, heavy and slow.
     (The DOM reads uCursorV for its own tilt; the field only glows.) */
  float cd = distance(uv * asp, uCursorV * asp);
  col += vec3(0.045, 0.05, 0.065) * smoothstep(0.55, 0.0, cd);

  /* t5 — grade: a soft vignette and a whisper of chromatic split at the
     edges, the way light fringes through thick glass. */
  float vig = smoothstep(1.25, 0.35, length(uv - 0.5) * 1.6);
  col *= 0.82 + 0.18 * vig;
  col.r += (1.0 - vig) * 0.012;
  col.b += (1.0 - vig) * 0.010;

  gl_FragColor = vec4(col, 1.0);
}
`

  /* ------------------------------------------------------------ plumbing */

  function compile(type, src) {
    const s = gl.createShader(type)
    gl.shaderSource(s, src)
    gl.compileShader(s)
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
      console.error('[egl]', gl.getShaderInfoLog(s))
      return null
    }
    return s
  }
  function program(fsSrc) {
    const p = gl.createProgram()
    gl.attachShader(p, compile(gl.VERTEX_SHADER, QUAD_VS))
    gl.attachShader(p, compile(gl.FRAGMENT_SHADER, fsSrc))
    gl.linkProgram(p)
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
      console.error('[egl]', gl.getProgramInfoLog(p))
      return null
    }
    return p
  }

  const quad = gl.createBuffer()
  gl.bindBuffer(gl.ARRAY_BUFFER, quad)
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW)

  const simProg = program(SIM_FS)
  const paintProg = program(PAINT_FS)
  if (!simProg || !paintProg) return { mount: () => {}, ok: false }

  const extHF = gl.getExtension('OES_texture_half_float')
  const extHFL = gl.getExtension('OES_texture_half_float_linear')
  const FMT = extHF ? extHF.HALF_FLOAT_OES : null

  // Simulation resolution: pattern lives at ~1/6 of screen and is bilinearly
  // magnified by the paint pass — softness is free, and the chemistry's
  // heavy inertia comes for free too.
  let SW = 0, SH = 0
  let texA = null, texB = null, fbA = null, fbB = null

  function makeTarget(w, h) {
    const tex = gl.createTexture()
    gl.bindTexture(gl.TEXTURE_2D, tex)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, extHFL ? gl.LINEAR : gl.NEAREST)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, extHFL ? gl.LINEAR : gl.NEAREST)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
    if (FMT) gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, FMT, null)
    else gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null)
    const fb = gl.createFramebuffer()
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb)
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0)
    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
    return { tex, fb }
  }

  function seedField(w, h) {
    // U = 1 everywhere, V scattered in sparse clumps — the chemistry's
    // starting culture.
    const px = new Uint8Array(w * h * 4)
    for (let i = 0; i < w * h; i++) {
      px[i * 4 + 0] = 255
      px[i * 4 + 1] = Math.random() < 0.018 ? 200 + (Math.random() * 55) | 0 : 0
      px[i * 4 + 2] = 0
      px[i * 4 + 3] = 255
    }
    for (const t of [texA, texB]) {
      gl.bindTexture(gl.TEXTURE_2D, t.tex)
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, px)
    }
  }

  function alloc(w, h) {
    SW = w; SH = h
    texA && gl.deleteTexture(texA.tex); texB && gl.deleteTexture(texB.tex)
    fbA && gl.deleteFramebuffer(fbA); fbB && gl.deleteFramebuffer(fbB)
    texA = makeTarget(w, h); fbA = texA.fb
    texB = makeTarget(w, h); fbB = texB.fb
    seedField(w, h)
  }

  const loc = (p, n) => gl.getUniformLocation(p, n)
  const simU = {
    prev: loc(simProg, 'uPrev'), texel: loc(simProg, 'uTexel'),
    feed: loc(simProg, 'uFeed'), kill: loc(simProg, 'uKill'),
    seed: loc(simProg, 'uSeed'), seedAmt: loc(simProg, 'uSeedAmt'),
    seedRad: loc(simProg, 'uSeedRad'), time: loc(simProg, 'uTime'),
  }
  const paintU = {
    sim: loc(paintProg, 'uSim'), res: loc(paintProg, 'uRes'),
    time: loc(paintProg, 'uTime'), hashRate: loc(paintProg, 'uHashRate'),
    cursor: loc(paintProg, 'uCursor'), cursorV: loc(paintProg, 'uCursorV'),
    feed: loc(paintProg, 'uFeed'),
  }

  /* -------------------------------------------------------------- runtime */

  const state = {
    feed: 0.0545,        // overwritten from the executed Piet program
    kill: 0.0620,
    seed: [0.5, 0.5],
    seedAmt: 0,
    hashRate: 0,
    cursor: [0.5, 0.5],
    running: false,
    mounted: false,
  }

  const reduced = matchMedia('(prefers-reduced-motion: reduce)')

  function resize() {
    const dpr = Math.min(devicePixelRatio || 1, 1.6)
    const w = Math.round(innerWidth * dpr), h = Math.round(innerHeight * dpr)
    if (canvas.width === w && canvas.height === h) return
    canvas.width = w; canvas.height = h
    alloc(Math.max(64, w >> 4), Math.max(64, h >> 4))
  }
  addEventListener('resize', resize)

  let last = 0
  function frame(now) {
    if (!state.running) return
    requestAnimationFrame(frame)
    // Sim advances at a fixed 30 Hz against the paint's rAF — the heavy
    // inertia is in the math; the clock just keeps it honest.
    const doSim = now - last > 33
    if (doSim) last = now

    if (doSim && !reduced.matches) {
      // A few relaxation steps per tick so the labyrinth keeps breathing
      // even without traffic.
      for (let i = 0; i < 6; i++) {
        gl.useProgram(simProg)
        gl.bindFramebuffer(gl.FRAMEBUFFER, fbB)
        gl.viewport(0, 0, SW, SH)
        gl.activeTexture(gl.TEXTURE0)
        gl.bindTexture(gl.TEXTURE_2D, texA.tex)
        gl.uniform1i(simU.prev, 0)
        gl.uniform2f(simU.texel, 1 / SW, 1 / SH)
        gl.uniform1f(simU.feed, state.feed)
        gl.uniform1f(simU.kill, state.kill)
        gl.uniform2f(simU.seed, state.seed[0], state.seed[1])
        gl.uniform1f(simU.seedAmt, state.seedAmt)
        gl.uniform1f(simU.seedRad, 0.045)
        gl.uniform1f(simU.time, now / 1000)
        gl.bindBuffer(gl.ARRAY_BUFFER, quad)
        const aP = gl.getAttribLocation(simProg, 'aP')
        gl.enableVertexAttribArray(aP)
        gl.vertexAttribPointer(aP, 2, gl.FLOAT, false, 0, 0)
        gl.drawArrays(gl.TRIANGLES, 0, 3)
        // ping-pong
        const tt = texA; texA = texB; texB = tt
        const tf = fbA; fbA = fbB; fbB = tf
        // a disturbance is a dose, not a drip: decay the syringe
        state.seedAmt *= 0.82
      }
    }

    gl.useProgram(paintProg)
    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
    gl.viewport(0, 0, canvas.width, canvas.height)
    gl.activeTexture(gl.TEXTURE0)
    gl.bindTexture(gl.TEXTURE_2D, texA.tex)
    gl.uniform1i(paintU.sim, 0)
    gl.uniform2f(paintU.res, canvas.width, canvas.height)
    gl.uniform1f(paintU.time, now / 1000)
    gl.uniform1f(paintU.hashRate, state.hashRate)
    gl.uniform1f(paintU.cursor, state.cursor[0])
    gl.uniform2f(paintU.cursorV, state.cursor[0], state.cursor[1])
    gl.uniform1f(paintU.feed, state.feed)
    const aP = gl.getAttribLocation(paintProg, 'aP')
    gl.enableVertexAttribArray(aP)
    gl.vertexAttribPointer(aP, 2, gl.FLOAT, false, 0, 0)
    gl.drawArrays(gl.TRIANGLES, 0, 3)
  }

  return {
    ok: true,
    mount(parent) {
      if (state.mounted) return
      state.mounted = true
      parent.prepend(canvas)
      resize()
      state.running = true
      requestAnimationFrame(frame)
    },
    /** Disturb the chemistry at a field position. `amt` 0..1. */
    disturb(x, y, amt = 0.8) {
      state.seed = [x, 1 - y]
      state.seedAmt = Math.min(1, state.seedAmt + amt)
    },
    /** The executed Piet program's output sets the feed rate. */
    setFeed(f) { state.feed = 0.037 + 0.03 * Math.min(1, Math.max(0, f)) },
    /** Events per second — drives the shimmer sweep. */
    setHashRate(r) { state.hashRate = Math.min(1, r) },
    /** Cursor in field space; the field leans toward the hand. */
    setCursor(x, y) { state.cursor = [x, 1 - y] },
  }
})()

// A top-level `const` does not reach `window`; the field is the one global
// the renderer legitimately shares, so it is handed over explicitly.
window.EGL = EGL
