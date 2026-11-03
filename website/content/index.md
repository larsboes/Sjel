# The Sjel documentation

<p class="lede-docs">Your gateway to the sovereign personal system for local memory, feeds, and native client apps.</p>

If you'd like to familiarize yourself with Sjel's core design and boundary invariants, we recommend reading the architecture overview:

<a class="action-card" href="architecture/index.md">
  <div class="card-main">
    <strong>Architecture overview</strong>
    <span class="desc">The Spine, capabilities, Packs, storage boundaries, and data processing policies.</span>
  </div>
  <span class="chevron" aria-hidden="true">→</span>
</a>

If you want to set up and run your own sovereign node right away, we recommend the minimal shell guide:

<a class="action-card" href="operations/index.md#install-a-shell-only-node">
  <div class="card-main">
    <strong>Quickstart: Shell-only node</strong>
    <span class="desc">Two-minute setup, installation footprint, and service persistence on macOS or Linux.</span>
  </div>
  <span class="chevron" aria-hidden="true">→</span>
</a>

## Sections

<div class="section-grid">
  <a class="section-card" href="architecture/index.md">
    <span>Architecture</span>
    <span class="chevron" aria-hidden="true">→</span>
  </a>
  <a class="section-card" href="capabilities/index.md">
    <span>Capabilities</span>
    <span class="chevron" aria-hidden="true">→</span>
  </a>
  <a class="section-card" href="operations/index.md">
    <span>Operations</span>
    <span class="chevron" aria-hidden="true">→</span>
  </a>
  <a class="section-card" href="../reference/index.html">
    <span>API Reference</span>
    <span class="chevron" aria-hidden="true">→</span>
  </a>
</div>

## Task-based guides

Choose a guide by the task you want to complete:

<div class="guide-grid">
  <div class="guide-card">
    <div class="guide-tag">Release & Friends</div>
    <h4>Prepare a release or friend trial</h4>
    <div class="guide-actions">
      <a class="btn-guide primary" href="operations/index.md#release-preparation-and-later-testing">Release preparation →</a>
      <a class="btn-guide secondary" href="https://github.com/larsboes/Sjel/issues/235">Open acceptance work (#235)</a>
    </div>
  </div>

  <div class="guide-card">
    <div class="guide-tag">Setup & Run</div>
    <h4>Install and verify a minimal shell</h4>
    <div class="guide-actions">
      <a class="btn-guide primary" href="operations/index.md#install-a-shell-only-node">Shell-only node →</a>
      <a class="btn-guide secondary" href="operations/persistence.md">Footprint & persistence</a>
    </div>
  </div>

  <div class="guide-card">
    <div class="guide-tag">System Design</div>
    <h4>Understand where a component belongs</h4>
    <div class="guide-actions">
      <a class="btn-guide primary" href="architecture/index.md">Architecture overview →</a>
      <a class="btn-guide secondary" href="architecture/storage.md">Storage & data classes</a>
    </div>
  </div>

  <div class="guide-card">
    <div class="guide-tag">Configuration</div>
    <h4>Design the optional feature manager</h4>
    <div class="guide-actions">
      <a class="btn-guide primary" href="architecture/feature-manager.md">Feature manager →</a>
      <a class="btn-guide secondary" href="operations/index.md#release-preparation-and-later-testing">Release preparation</a>
    </div>
  </div>

  <div class="guide-card">
    <div class="guide-tag">Capabilities</div>
    <h4>Add or change a capability</h4>
    <div class="guide-actions">
      <a class="btn-guide primary" href="capabilities/manifest.md">Capability manifests →</a>
      <a class="btn-guide secondary" href="capabilities/contracts.md">Health & freshness contracts</a>
    </div>
  </div>

  <div class="guide-card">
    <div class="guide-tag">Host & Ops</div>
    <h4>Diagnose or manage a host</h4>
    <div class="guide-actions">
      <a class="btn-guide primary" href="operations/index.md">Operations guide →</a>
      <a class="btn-guide secondary" href="operations/doctor.md">Doctor diagnostics suite</a>
    </div>
  </div>

  <div class="guide-card">
    <div class="guide-tag">Clients & UI</div>
    <h4>Projects & Generative UI</h4>
    <div class="guide-actions">
      <a class="btn-guide primary" href="architecture/projects-ui.md">Projects UI →</a>
      <a class="btn-guide secondary" href="../../CONTRIBUTING.md#product-rules">Engineering doctrine</a>
    </div>
  </div>

  <div class="guide-card">
    <div class="guide-tag">Disaster Recovery</div>
    <h4>Rehearse recovery before a migration</h4>
    <div class="guide-actions">
      <a class="btn-guide primary" href="architecture/storage.md#backups-before-migrations">Storage and recovery →</a>
      <a class="btn-guide secondary" href="capabilities/index.md">The owning capability's contract</a>
    </div>
  </div>
</div>

<div class="note">
Repository doctrine lives in <a href="../../CONTRIBUTING.md#engineering-doctrine">CONTRIBUTING.md</a>; the declared structure is generated in <a href="../../ARCHITECTURE.md">ARCHITECTURE.md</a>. Open work and priorities live in <a href="../../PLAN.md">PLAN.md</a>. Read the owning capability or Pack README for its purpose and design choices.
</div>
