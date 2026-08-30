<script setup lang="ts">
// Decorative 3D shapes that float in the hero background
</script>

<template>
  <div class="float-shapes pointer-events-none" aria-hidden="true">
    <!-- Rotating wireframe cube -->
    <div class="shape cube">
      <div class="face f-front" />
      <div class="face f-back" />
      <div class="face f-right" />
      <div class="face f-left" />
      <div class="face f-top" />
      <div class="face f-bottom" />
    </div>

    <!-- Tilted ring -->
    <div class="shape ring-1" />

    <!-- Smaller secondary ring -->
    <div class="shape ring-2" />

    <!-- Floating sphere (CSS gradient) -->
    <div class="shape sphere" />
  </div>
</template>

<style scoped>
.float-shapes {
  position: absolute;
  inset: 0;
  perspective: 1600px;
  z-index: 1;
  overflow: hidden;
}

.shape {
  position: absolute;
  transform-style: preserve-3d;
}

/* ── Cube ── */
.cube {
  width: 80px;
  height: 80px;
  top: 16%;
  right: 14%;
  animation: rotateCube 24s linear infinite;
}
.cube .face {
  position: absolute;
  width: 80px;
  height: 80px;
  border: 1px solid rgba(255, 255, 255, 0.4);
  background: linear-gradient(135deg, rgba(255, 255, 255, 0.12), rgba(255, 255, 255, 0.02));
  box-shadow: inset 0 0 24px rgba(255, 255, 255, 0.18);
}
.f-front  { transform: translateZ(40px); }
.f-back   { transform: rotateY(180deg) translateZ(40px); }
.f-right  { transform: rotateY(90deg) translateZ(40px); }
.f-left   { transform: rotateY(-90deg) translateZ(40px); }
.f-top    { transform: rotateX(90deg) translateZ(40px); }
.f-bottom { transform: rotateX(-90deg) translateZ(40px); }

@keyframes rotateCube {
  0%   { transform: rotateX(-20deg) rotateY(0deg); }
  100% { transform: rotateX(-20deg) rotateY(360deg); }
}

/* ── Rings ── */
.ring-1 {
  width: 240px;
  height: 240px;
  bottom: 12%;
  left: 5%;
  border: 1px solid rgba(255, 255, 255, 0.22);
  border-radius: 50%;
  animation: floatRing1 18s ease-in-out infinite;
  box-shadow: 0 0 80px rgba(255, 255, 255, 0.12), inset 0 0 80px rgba(255, 255, 255, 0.08);
}
.ring-2 {
  width: 150px;
  height: 150px;
  top: 38%;
  right: 24%;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 50%;
  animation: floatRing2 24s ease-in-out infinite reverse;
  box-shadow: 0 0 60px rgba(255, 255, 255, 0.05);
}

@keyframes floatRing1 {
  0%, 100% { transform: rotateX(70deg) rotateZ(0deg) translateY(0); }
  50%      { transform: rotateX(70deg) rotateZ(180deg) translateY(-30px); }
}
@keyframes floatRing2 {
  0%, 100% { transform: rotateX(70deg) rotateZ(0deg) translateY(0); }
  50%      { transform: rotateX(70deg) rotateZ(-180deg) translateY(20px); }
}

/* ── Sphere ── */
.sphere {
  width: 120px;
  height: 120px;
  bottom: 30%;
  right: 6%;
  border-radius: 50%;
  background: radial-gradient(
    circle at 30% 30%,
    rgba(255, 255, 255, 0.45),
    rgba(255, 255, 255, 0.08) 50%,
    transparent 75%
  );
  box-shadow: 0 0 80px rgba(255, 255, 255, 0.25), inset 0 0 40px rgba(255, 255, 255, 0.15);
  animation: floatSphere 12s ease-in-out infinite;
}
@keyframes floatSphere {
  0%, 100% { transform: translateY(0) scale(1); }
  50%      { transform: translateY(-25px) scale(1.05); }
}

@media (max-width: 768px) {
  .cube, .ring-1, .ring-2, .sphere { display: none; }
}
</style>
