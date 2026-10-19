<script lang="ts">
  let { title }: { title?: string } = $props();

  type Subsystem = 'all' | 'power' | 'water' | 'drive' | 'drain' | 'safety';

  let activeSystem = $state<Subsystem>('all');
  let selectedPart = $state<string>('filter');

  interface PartInfo {
    id: string;
    title: string;
    subtitle: string;
    location: string;
    note: string;
    noteLink: string;
    details: string;
    check?: string;
  }

  const parts: Record<string, PartInfo> = {
    filter: {
      id: 'filter',
      title: 'Mains Filter (Netzfilter)',
      subtitle: 'Part No. 4364110 · BV 16.650/3',
      location: 'Rear wall bottom opening, hooks into frame top, 1 screw at bottom',
      note: 'Washing Machine Electrics',
      noteLink: '/projects?project=Washing%20Machine&open=Washing%20Machine%2FWashing%20Machine%20Electrics.md',
      details: '470nF X1 + 2x15nF Y2 + 680kΩ. Takes the 230V mains cable and distributes to White plug (2814), Black plug, and PE ground bracket.',
      check: 'M1: Ground bracket continuity to chassis (< 0.2 Ω). Ground bracket holder was broken, re-seated.',
    },
    plugs: {
      id: 'plugs',
      title: 'Internal Harness Plugs',
      subtitle: 'White Plug "2814" & Black Plug',
      location: 'Plugged into the two keyed front slots of the mains filter',
      note: 'Washing Machine Electrics',
      noteLink: '/projects?project=Washing%20Machine&open=Washing%20Machine%2FWashing%20Machine%20Electrics.md',
      details: 'White plug "2814" on black cable feeds the upper electronic control unit. Black plug feeds motor/heater power relays. Ground wire tab feeds chassis PE.',
      check: 'Tabs A and B remain empty. Never connect anything to tabs A & B.',
    },
    chassis: {
      id: 'chassis',
      title: 'Chassis & Rear Wall Opening',
      subtitle: '60 × 60 × 85 cm enamelled steel casing',
      location: 'Bottom rear service opening',
      note: 'Washing Machine Connect and Test',
      noteLink: '/projects?project=Washing%20Machine&open=Washing%20Machine%2FWashing%20Machine%20Connect%20and%20Test.md',
      details: 'All metallic panels, tub ground strap, and outer frame form the protective earth (PE) cage.',
      check: 'Step 6: Mount the filter in the frame: hook in at top, secure screw at bottom. No pinched wires.',
    },
    electronics: {
      id: 'electronics',
      title: 'Power & Control Electronics (EDFU / EL)',
      subtitle: 'Program timer, motor speed control & user console',
      location: 'Upper front fascia behind selector dial and pushbuttons',
      note: 'Washing Machine Electrics',
      noteLink: '/projects?project=Washing%20Machine&open=Washing%20Machine%2FWashing%20Machine%20Electrics.md',
      details: 'Fed by the white cable "2814". Controls water inlet solenoids, heater relay, motor inverter/triac, and door latch lock.',
      check: 'Optical "PC" port on panel is Miele diagnostic service interface only.',
    },
    inlet: {
      id: 'inlet',
      title: 'Water Inlet Valve (Einlassventil)',
      subtitle: 'Cold water solenoid valve · 10 bar rated',
      location: 'Top rear corner',
      note: 'Washing Machine Water and Pump',
      noteLink: '/projects?project=Washing%20Machine&open=Washing%20Machine%2FWashing%20Machine%20Water%20and%20Pump.md',
      details: 'Connects to cold water tap via 3/4" union nut with strainer. Directs flow into pre-wash or main wash detergent compartments.',
      check: 'Inlet hose marked "25 °C MAX". Tighten union nut by hand + 1/4 turn, check for leaks under tap pressure.',
    },
    drum: {
      id: 'drum',
      title: 'Wash Drum & Outer Tub (Laugenbehälter)',
      subtitle: 'Stainless steel drum (1400 rpm max) in suspended suds container',
      location: 'Central floating assembly',
      note: 'Washing Machine',
      noteLink: '/projects?project=Washing%20Machine&open=Washing%20Machine%2FWashing%20Machine.md',
      details: 'Suspended by 4 heavy coil springs at the top and dampened by 2 hydraulic shock absorbers at the bottom. Carries front and top cast-iron weights.',
      check: 'Check free rotation by hand with power off; listen for bearing rumble or trapped debris.',
    },
    motor: {
      id: 'motor',
      title: 'Drive Motor & Belt',
      subtitle: 'Kollektormotor with tachogenerator & ribbed V-belt',
      location: 'Bolted to bottom of tub beneath drum axis',
      note: 'Washing Machine Electrics',
      noteLink: '/projects?project=Washing%20Machine&open=Washing%20Machine%2FWashing%20Machine%20Electrics.md',
      details: 'Drives the large rear drum pulley wheel up to 1400 rpm. Green internal ground strap connects motor frame to main chassis.',
      check: 'Check carbon brushes if motor fails to turn during empty test run.',
    },
    heater: {
      id: 'heater',
      title: 'Tubular Heater & NTC Sensor',
      subtitle: '2000 W immersion element with temperature sensor',
      location: 'Inserted into bottom rear of suds container below drum',
      note: 'Washing Machine Electrics',
      noteLink: '/projects?project=Washing%20Machine&open=Washing%20Machine%2FWashing%20Machine%20Electrics.md',
      details: 'Heats wash water to selected temperature (30°C to 95°C). Governed by control board via NTC sensor.',
      check: 'M2: Insulation resistance heater-to-PE > 2 MΩ with multimeter before running.',
    },
    pump: {
      id: 'pump',
      title: 'Drain Pump & Lint Filter (Laugenpumpe & Flusensieb)',
      subtitle: 'Synchronous pump motor with impeller',
      location: 'Bottom front left behind hinged service flap',
      note: 'Washing Machine Water and Pump',
      noteLink: '/projects?project=Washing%20Machine&open=Washing%20Machine%2FWashing%20Machine%20Water%20and%20Pump.md',
      details: 'Pumps suds out of tub through lint trap to rear drain spigot. Emergency drain tube located next to filter handle.',
      check: 'Tip from Anna: Open filter, place flat bowl beneath, check impeller turns freely by hand. If stuck, spray with water before powering.',
    },
    drainHose: {
      id: 'drainHose',
      title: 'Drain Spigot & Hose',
      subtitle: 'Part No. 1995932 with yellow spigot ring',
      location: 'Bottom rear spigot -> wall siphon nipple',
      note: 'Washing Machine Water and Pump',
      noteLink: '/projects?project=Washing%20Machine&open=Washing%20Machine%2FWashing%20Machine%20Water%20and%20Pump.md',
      details: 'Original Miele hose ordered on eBay (17.10.2026). Hose sits over spigot and clamp secures it. Routes to wall drain nipple.',
      check: 'Max drain pump height: 1.0 m ("Die maximale Abpumphöhe beträgt 1 m"). Do not kink.',
    },
  };

  const currentInfo = $derived(parts[selectedPart] ?? parts.filter);

  function pickSystem(sys: Subsystem) {
    activeSystem = sys;
    if (sys === 'power') selectedPart = 'filter';
    else if (sys === 'water') selectedPart = 'inlet';
    else if (sys === 'drive') selectedPart = 'drum';
    else if (sys === 'drain') selectedPart = 'pump';
    else if (sys === 'safety') selectedPart = 'filter';
  }

  function isHighlighted(partId: string): boolean {
    if (activeSystem === 'all') return true;
    if (activeSystem === 'power') return ['filter', 'plugs', 'chassis', 'electronics'].includes(partId);
    if (activeSystem === 'water') return ['inlet', 'drum', 'heater', 'pump', 'drainHose'].includes(partId);
    if (activeSystem === 'drive') return ['drum', 'motor', 'chassis'].includes(partId);
    if (activeSystem === 'drain') return ['drum', 'pump', 'drainHose'].includes(partId);
    if (activeSystem === 'safety') return ['filter', 'chassis', 'heater', 'pump'].includes(partId);
    return true;
  }
</script>

<div class="machine-system-card">
  <div class="header-bar">
    <div class="title-cluster">
      <span class="badge">Miele Gala W 961</span>
      <h3>{title ?? "Complete Machine System & Connections"}</h3>
    </div>
    <div class="subsystem-nav" role="tablist" aria-label="Washing machine subsystems">
      <button
        type="button"
        class="nav-btn"
        class:active={activeSystem === 'all'}
        onclick={() => pickSystem('all')}
        role="tab"
        aria-selected={activeSystem === 'all'}
      >All Systems</button>
      <button
        type="button"
        class="nav-btn"
        class:active={activeSystem === 'power'}
        onclick={() => pickSystem('power')}
        role="tab"
        aria-selected={activeSystem === 'power'}
      >Power & Filter</button>
      <button
        type="button"
        class="nav-btn"
        class:active={activeSystem === 'water'}
        onclick={() => pickSystem('water')}
        role="tab"
        aria-selected={activeSystem === 'water'}
      >Water Circuit</button>
      <button
        type="button"
        class="nav-btn"
        class:active={activeSystem === 'drive'}
        onclick={() => pickSystem('drive')}
        role="tab"
        aria-selected={activeSystem === 'drive'}
      >Drive & Drum</button>
      <button
        type="button"
        class="nav-btn"
        class:active={activeSystem === 'drain'}
        onclick={() => pickSystem('drain')}
        role="tab"
        aria-selected={activeSystem === 'drain'}
      >Drain & Pump</button>
      <button
        type="button"
        class="nav-btn"
        class:active={activeSystem === 'safety'}
        onclick={() => pickSystem('safety')}
        role="tab"
        aria-selected={activeSystem === 'safety'}
      >Safety & M1–M5</button>
    </div>
  </div>

  <div class="figure-container">
    <div class="svg-stage">
      <svg
        viewBox="0 0 860 560"
        class="schematic-svg"
        xmlns="http://www.w3.org/2000/svg"
        role="img"
        aria-label="Interactive isometric cutaway of Miele Gala W 961 washing machine showing all components and connections"
      >
        <defs>
          <linearGradient id="cabGrad" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stop-color="var(--surface, #f8fafc)" stop-opacity="0.8" />
            <stop offset="100%" stop-color="var(--rule, #e2e8f0)" stop-opacity="0.3" />
          </linearGradient>
          <linearGradient id="drumGrad" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stop-color="#94a3b8" stop-opacity="0.6" />
            <stop offset="100%" stop-color="#475569" stop-opacity="0.8" />
          </linearGradient>
          <linearGradient id="powerGrad" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stop-color="#eab308" />
            <stop offset="100%" stop-color="#ca8a04" />
          </linearGradient>
          <linearGradient id="waterGrad" x1="0%" y1="0%" x2="0%" y2="100%">
            <stop offset="0%" stop-color="#38bdf8" />
            <stop offset="100%" stop-color="#0284c7" />
          </linearGradient>
          <linearGradient id="drainGrad" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stop-color="#a855f7" />
            <stop offset="100%" stop-color="#7e22ce" />
          </linearGradient>
          <filter id="glow">
            <feGaussianBlur stdDeviation="3" result="blur" />
            <feComposite in="SourceGraphic" in2="blur" operator="over" />
          </filter>
        </defs>

        <!-- Base ground shadow -->
        <ellipse cx="430" cy="510" rx="340" ry="40" fill="currentColor" opacity="0.06" />

        <!-- 1. OUTER CABINET (Transparent wireframe enclosure in isometric perspective) -->
        <g class="cabinet" opacity={isHighlighted('chassis') ? "1" : "0.3"}>
          <!-- Back Wall -->
          <polygon points="200,120 540,70 540,430 200,480" fill="url(#cabGrad)" stroke="var(--rule, #cbd5e1)" stroke-width="1.5" />
          <!-- Base Pan -->
          <polygon points="200,480 540,430 680,480 340,530" fill="var(--surface, #f1f5f9)" stroke="var(--rule, #94a3b8)" stroke-width="2" />
          <!-- Right Side Wall -->
          <polygon points="540,70 680,120 680,480 540,430" fill="url(#cabGrad)" stroke="var(--rule, #cbd5e1)" stroke-width="1.5" stroke-dasharray="3,3" />
          <!-- Top Cover Panel -->
          <polygon points="200,120 340,170 680,120 540,70" fill="none" stroke="var(--rule, #94a3b8)" stroke-width="2" />
          <!-- Left Front Wall Corner -->
          <line x1="200" y1="120" x2="200" y2="480" stroke="var(--rule, #94a3b8)" stroke-width="2" />
          <!-- Front Corner -->
          <line x1="340" y1="170" x2="340" y2="530" stroke="var(--rule, #94a3b8)" stroke-width="2" />
          <!-- Right Corner -->
          <line x1="680" y1="120" x2="680" y2="480" stroke="var(--rule, #94a3b8)" stroke-width="2" />
        </g>

        <!-- 2. REAR ACCESS OPENING (Where mains filter installs) -->
        <g class="rear-opening" class:active={selectedPart === 'filter' || selectedPart === 'plugs'}>
          <polygon points="230,360 310,348 310,448 230,460" fill="none" stroke="var(--primary, #2563eb)" stroke-width="2" stroke-dasharray="4,4" />
          <text x="220" y="340" fill="var(--primary, #2563eb)" font-size="11" font-weight="600">Rear Opening</text>
        </g>

        <!-- 3. WATER INLET SYSTEM (Top Rear) -->
        <g class="component-group water-inlet" opacity={isHighlighted('inlet') ? "1" : "0.2"}>
          <!-- Inlet Hose from tap -->
          <path d="M120,60 Q170,50 240,90 L270,110" fill="none" stroke="#0284c7" stroke-width="5" stroke-linecap="round" />
          <path d="M120,60 Q170,50 240,90 L270,110" fill="none" stroke="#38bdf8" stroke-width="2" stroke-dasharray="8,6" />
          <!-- Solenoid Valve Block -->
          <rect x="265" y="100" width="30" height="24" rx="3" fill="#0284c7" stroke="#0369a1" stroke-width="1.5" />
          <!-- Tubes to detergent drawer -->
          <path d="M295,112 Q330,115 380,150" fill="none" stroke="#38bdf8" stroke-width="3" stroke-dasharray="3,3" />
          <!-- Detergent Drawer Housing -->
          <polygon points="360,165 420,155 420,195 360,205" fill="#f8fafc" stroke="#94a3b8" stroke-width="1.5" />
          <text x="365" y="185" font-size="9" fill="#64748b">Drawer</text>
        </g>

        <!-- 4. CONTROL ELECTRONICS & CONSOLE (Top Front) -->
        <g class="component-group electronics" opacity={isHighlighted('electronics') ? "1" : "0.25"}>
          <!-- Fascia console panel -->
          <polygon points="340,170 480,150 480,210 340,230" fill="var(--card-bg, #ffffff)" stroke="#64748b" stroke-width="1.5" />
          <!-- Program Dial -->
          <ellipse cx="440" cy="180" rx="14" ry="10" fill="#cbd5e1" stroke="#475569" stroke-width="1.5" />
          <!-- Microprocessor Board inside -->
          <polygon points="350,180 470,160 460,200 350,215" fill="#15803d" opacity="0.85" stroke="#166534" stroke-width="1" />
        </g>

        <!-- 5. SUSPENSION & DRUM ASSEMBLY -->
        <g class="component-group drive-assembly" opacity={isHighlighted('drum') || isHighlighted('motor') ? "1" : "0.25"}>
          <!-- Top suspension springs -->
          <path d="M300,135 Q305,170 315,200" fill="none" stroke="#64748b" stroke-width="2" stroke-dasharray="2,3" />
          <path d="M520,105 Q515,140 500,180" fill="none" stroke="#64748b" stroke-width="2" stroke-dasharray="2,3" />

          <!-- Outer Suds Container (Tub) -->
          <ellipse cx="420" cy="300" rx="110" ry="110" fill="url(#drumGrad)" stroke="#334155" stroke-width="2" />
          <!-- Honeycomb Drum Front Opening -->
          <ellipse cx="400" cy="310" rx="75" ry="75" fill="#0f172a" opacity="0.75" stroke="#cbd5e1" stroke-width="3" />
          <ellipse cx="400" cy="310" rx="50" ry="50" fill="none" stroke="#94a3b8" stroke-width="1.5" stroke-dasharray="4,4" />
          <!-- Door Glass Outline -->
          <ellipse cx="370" cy="325" rx="70" ry="70" fill="none" stroke="#38bdf8" stroke-width="1.5" opacity="0.5" />

          <!-- Lower Shock Absorbers (Stoßdämpfer) -->
          <line x1="350" y1="390" x2="310" y2="480" stroke="#475569" stroke-width="6" stroke-linecap="round" />
          <line x1="490" y1="390" x2="520" y2="450" stroke="#475569" stroke-width="6" stroke-linecap="round" />

          <!-- Drive Motor (Under Tub) -->
          <g class="motor-unit" class:active={selectedPart === 'motor'}>
            <polygon points="410,400 480,390 480,440 410,450" fill="#334155" stroke="#1e293b" stroke-width="2" />
            <ellipse cx="410" cy="425" rx="8" ry="15" fill="#64748b" stroke="#334155" stroke-width="1.5" />
            <!-- Drive Belt -->
            <path d="M410,415 Q430,350 490,260" fill="none" stroke="#0f172a" stroke-width="3.5" />
            <text x="425" y="425" font-size="10" fill="#f8fafc" font-weight="600">Motor</text>
          </g>

          <!-- Tubular Heater (Base of Tub) -->
          <path d="M370,385 L440,375 L450,380 L380,390" fill="none" stroke="#ef4444" stroke-width="3.5" stroke-linecap="round" />
        </g>

        <!-- 6. DRAIN SYSTEM (Tub Sump -> Pump -> Drain Spigot) -->
        <g class="component-group drain-system" opacity={isHighlighted('drain') || isHighlighted('drainHose') ? "1" : "0.25"}>
          <!-- Sump Hose from tub bottom -->
          <path d="M400,390 Q390,440 370,470" fill="none" stroke="#475569" stroke-width="8" stroke-linecap="round" />
          <!-- Drain Pump Body (Front Bottom Left) -->
          <g class="pump-unit" class:active={selectedPart === 'pump'}>
            <rect x="340" y="460" width="40" height="32" rx="4" fill="#6b21a8" stroke="#581c87" stroke-width="2" />
            <!-- Impeller representation -->
            <circle cx="360" cy="476" r="9" fill="#c084fc" opacity="0.8" />
            <line x1="355" y1="476" x2="365" y2="476" stroke="#581c87" stroke-width="2" />
            <line x1="360" y1="471" x2="360" y2="481" stroke="#581c87" stroke-width="2" />
            <text x="345" y="452" font-size="9" fill="#a855f7" font-weight="600">Drain Pump</text>
          </g>
          <!-- Drain Hose to rear spigot -->
          <path d="M375,475 Q320,490 260,450" fill="none" stroke="#9333ea" stroke-width="4.5" stroke-linecap="round" />
          <!-- Rear Drain Spigot (with yellow ring) -->
          <circle cx="260" cy="450" r="7" fill="#eab308" stroke="#ca8a04" stroke-width="2" />
          <!-- External Drain Hose 1995932 running outward -->
          <path d="M260,450 Q200,430 150,440 T80,420" fill="none" stroke="#7e22ce" stroke-width="6" stroke-linecap="round" stroke-dasharray="8,4" />
          <text x="80" y="410" font-size="10" fill="#7e22ce" font-weight="600">Drain Hose (1995932)</text>
        </g>

        <!-- 7. ELECTRICAL MAINS FILTER & WIRING HARNESS -->
        <g class="component-group power-filter" opacity={isHighlighted('filter') || isHighlighted('plugs') ? "1" : "0.25"}>
          <!-- 230V Schuko Mains Cable in from wall -->
          <path d="M70,390 Q140,410 240,410" fill="none" stroke="#475569" stroke-width="5" stroke-linecap="round" />
          <path d="M70,390 Q140,410 240,410" fill="none" stroke="#eab308" stroke-width="2" stroke-dasharray="6,4" />
          <text x="60" y="380" font-size="10" fill="#eab308" font-weight="600">230V Mains Cable</text>

          <!-- Mains Filter Box (Netzfilter 4364110) -->
          <g class="filter-box" class:active={selectedPart === 'filter'}>
            <polygon points="245,390 285,384 285,434 245,440" fill="#ffffff" stroke="#2563eb" stroke-width="2.5" />
            <!-- Slots and tabs -->
            <rect x="252" y="396" width="6" height="8" fill="#0284c7" />
            <rect x="264" y="394" width="6" height="8" fill="#1e293b" />
            <!-- Ground Bracket (Metal) -->
            <polygon points="275,392 283,390 283,405 275,407" fill="#22c55e" stroke="#15803d" stroke-width="1.5" />
            <text x="235" y="375" font-size="10" fill="#2563eb" font-weight="700">Filter 4364110</text>
          </g>

          <!-- White Cable (2814) -> Power to Electronics -->
          <path d="M255,396 Q240,280 360,200" fill="none" stroke="#ffffff" stroke-width="3" />
          <path d="M255,396 Q240,280 360,200" fill="none" stroke="#64748b" stroke-width="1" stroke-dasharray="3,3" />

          <!-- Black Cable -> Power to Relays / Heater -->
          <path d="M267,394 Q290,380 380,385" fill="none" stroke="#0f172a" stroke-width="3" />

          <!-- Green-Yellow PE Ground Receptacle -> Chassis & Motor Ground Strap -->
          <path d="M280,395 Q310,395 330,450" fill="none" stroke="#22c55e" stroke-width="3.5" stroke-linecap="round" />
          <path d="M330,450 Q360,460 410,440" fill="none" stroke="#16a34a" stroke-width="2.5" stroke-dasharray="4,2" />
          <circle cx="330" cy="450" r="4" fill="#22c55e" stroke="#15803d" stroke-width="1.5" />
          <text x="300" y="468" font-size="9" fill="#16a34a" font-weight="600">PE Ground (M1)</text>
        </g>

        <!-- 8. INTERACTIVE HOTSPOT PINS -->
        <g class="hotspots">
          <!-- Filter Hotspot -->
          <g class="pin" onclick={() => selectedPart = 'filter'} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') selectedPart = 'filter'; }} role="button" tabindex="0">
            <circle cx="265" cy="410" r="14" fill="#2563eb" opacity="0.25" />
            <circle cx="265" cy="410" r="8" fill="#2563eb" stroke="#ffffff" stroke-width="2" />
            <text x="265" y="413" text-anchor="middle" font-size="9" font-weight="700" fill="#ffffff">1</text>
          </g>

          <!-- Control Electronics Hotspot -->
          <g class="pin" onclick={() => selectedPart = 'electronics'} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') selectedPart = 'electronics'; }} role="button" tabindex="0">
            <circle cx="410" cy="180" r="14" fill="#0284c7" opacity="0.25" />
            <circle cx="410" cy="180" r="8" fill="#0284c7" stroke="#ffffff" stroke-width="2" />
            <text x="410" y="183" text-anchor="middle" font-size="9" font-weight="700" fill="#ffffff">2</text>
          </g>

          <!-- Water Inlet Hotspot -->
          <g class="pin" onclick={() => selectedPart = 'inlet'} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') selectedPart = 'inlet'; }} role="button" tabindex="0">
            <circle cx="280" cy="112" r="14" fill="#0284c7" opacity="0.25" />
            <circle cx="280" cy="112" r="8" fill="#0284c7" stroke="#ffffff" stroke-width="2" />
            <text x="280" y="115" text-anchor="middle" font-size="9" font-weight="700" fill="#ffffff">3</text>
          </g>

          <!-- Drum Hotspot -->
          <g class="pin" onclick={() => selectedPart = 'drum'} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') selectedPart = 'drum'; }} role="button" tabindex="0">
            <circle cx="400" cy="310" r="16" fill="#475569" opacity="0.3" />
            <circle cx="400" cy="310" r="9" fill="#475569" stroke="#ffffff" stroke-width="2" />
            <text x="400" y="313" text-anchor="middle" font-size="9" font-weight="700" fill="#ffffff">4</text>
          </g>

          <!-- Motor Hotspot -->
          <g class="pin" onclick={() => selectedPart = 'motor'} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') selectedPart = 'motor'; }} role="button" tabindex="0">
            <circle cx="445" cy="420" r="14" fill="#334155" opacity="0.25" />
            <circle cx="445" cy="420" r="8" fill="#334155" stroke="#ffffff" stroke-width="2" />
            <text x="445" y="423" text-anchor="middle" font-size="9" font-weight="700" fill="#ffffff">5</text>
          </g>

          <!-- Drain Pump Hotspot -->
          <g class="pin" onclick={() => selectedPart = 'pump'} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') selectedPart = 'pump'; }} role="button" tabindex="0">
            <circle cx="360" cy="475" r="14" fill="#9333ea" opacity="0.25" />
            <circle cx="360" cy="475" r="8" fill="#9333ea" stroke="#ffffff" stroke-width="2" />
            <text x="360" y="478" text-anchor="middle" font-size="9" font-weight="700" fill="#ffffff">6</text>
          </g>

          <!-- Drain Hose Hotspot -->
          <g class="pin" onclick={() => selectedPart = 'drainHose'} onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') selectedPart = 'drainHose'; }} role="button" tabindex="0">
            <circle cx="200" cy="435" r="14" fill="#7e22ce" opacity="0.25" />
            <circle cx="200" cy="435" r="8" fill="#7e22ce" stroke="#ffffff" stroke-width="2" />
            <text x="200" y="438" text-anchor="middle" font-size="9" font-weight="700" fill="#ffffff">7</text>
          </g>
        </g>
      </svg>
    </div>

    <!-- Interactive Component Readout Panel -->
    <div class="readout-card">
      <div class="readout-header">
        <span class="part-badge">Part Details</span>
        <h4>{currentInfo.title}</h4>
        <span class="part-subtitle">{currentInfo.subtitle}</span>
      </div>

      <div class="readout-body">
        <div class="meta-row">
          <span class="meta-label">Location</span>
          <span class="meta-val">{currentInfo.location}</span>
        </div>
        <div class="meta-row">
          <span class="meta-label">Function</span>
          <span class="meta-val">{currentInfo.details}</span>
        </div>
        {#if currentInfo.check}
          <div class="check-box">
            <strong>Verification / Test Gate:</strong>
            <p>{currentInfo.check}</p>
          </div>
        {/if}
      </div>

      <div class="readout-footer">
        <span class="note-ref">Source note:</span>
        <a href={currentInfo.noteLink} class="note-link">
          {currentInfo.note} →
        </a>
      </div>
    </div>
  </div>
</div>

<style>
  .machine-system-card {
    background: var(--card-bg, #ffffff);
    border: 1px solid var(--rule, #e2e8f0);
    border-radius: var(--radius-md, 8px);
    overflow: hidden;
    margin: 1rem 0 1.5rem;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.05);
  }

  .header-bar {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
    padding: 0.85rem 1rem;
    background: var(--surface, #f8fafc);
    border-bottom: 1px solid var(--rule, #e2e8f0);
  }

  .title-cluster {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  .title-cluster h3 {
    margin: 0;
    font-size: var(--text-base, 1rem);
    font-weight: 600;
  }

  .badge {
    background: var(--primary-soft, #eff6ff);
    color: var(--primary, #2563eb);
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    font-size: var(--text-xs, 0.75rem);
    font-weight: 600;
  }

  .subsystem-nav {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }

  .nav-btn {
    background: transparent;
    border: 1px solid var(--rule, #cbd5e1);
    color: var(--text-secondary, #64748b);
    padding: 0.25rem 0.6rem;
    border-radius: var(--radius-sm, 4px);
    font-size: var(--text-xs, 0.75rem);
    font-weight: 500;
    cursor: pointer;
    transition: background var(--motion-fast) ease, color var(--motion-fast) ease, border-color var(--motion-fast) ease;
  }

  .nav-btn:hover {
    background: var(--card-bg, #ffffff);
    color: var(--text-primary, #0f172a);
    border-color: var(--primary, #2563eb);
  }

  .nav-btn.active {
    background: var(--primary, #2563eb);
    color: #ffffff;
    border-color: var(--primary, #2563eb);
  }

  .figure-container {
    display: grid;
    grid-template-columns: 1fr minmax(18rem, 22rem);
    gap: 1rem;
    padding: 1rem;
    align-items: start;
  }

  @media (max-width: 860px) {
    .figure-container {
      grid-template-columns: 1fr;
    }
  }

  .svg-stage {
    background: var(--bg, #fdfdfd);
    border: 1px solid var(--rule, #f1f5f9);
    border-radius: var(--radius-sm, 6px);
    padding: 0.5rem;
  }

  .schematic-svg {
    width: 100%;
    height: auto;
    display: block;
  }

  .pin {
    cursor: pointer;
    transition: transform var(--motion-fast) ease;
  }

  .pin:hover {
    transform: scale(1.15);
    transform-origin: center;
  }

  .readout-card {
    background: var(--surface, #f8fafc);
    border: 1px solid var(--rule, #e2e8f0);
    border-radius: var(--radius-sm, 6px);
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .readout-header {
    border-bottom: 1px solid var(--rule, #e2e8f0);
    padding-bottom: 0.6rem;
  }

  .part-badge {
    text-transform: uppercase;
    font-size: var(--text-2xs);
    letter-spacing: 0.05em;
    font-weight: 700;
    color: var(--primary, #2563eb);
  }

  .readout-header h4 {
    margin: 0.2rem 0 0.1rem;
    font-size: var(--text-md, 1.1rem);
  }

  .part-subtitle {
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-secondary, #64748b);
  }

  .readout-body {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .meta-row {
    font-size: var(--text-sm, 0.85rem);
    line-height: 1.4;
  }

  .meta-label {
    display: block;
    font-size: var(--text-xs, 0.72rem);
    font-weight: 600;
    color: var(--text-secondary, #64748b);
    text-transform: uppercase;
  }

  .meta-val {
    color: var(--text-primary, #1e293b);
  }

  .check-box {
    margin-top: 0.4rem;
    padding: 0.6rem;
    background: var(--card-bg, #ffffff);
    border-left: 3px solid #eab308;
    border-radius: 4px;
    font-size: var(--text-xs, 0.75rem);
  }

  .check-box p {
    margin: 0.2rem 0 0;
    color: var(--text-primary, #1e293b);
  }

  .readout-footer {
    margin-top: auto;
    padding-top: 0.6rem;
    border-top: 1px solid var(--rule, #e2e8f0);
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: var(--text-xs, 0.75rem);
  }

  .note-ref {
    color: var(--text-secondary, #64748b);
  }

  .note-link {
    color: var(--primary, #2563eb);
    font-weight: 600;
    text-decoration: none;
  }

  .note-link:hover {
    text-decoration: underline;
  }
</style>
