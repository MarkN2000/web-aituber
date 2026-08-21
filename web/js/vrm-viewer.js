import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { VRMLoaderPlugin, VRMUtils } from '@pixiv/three-vrm';
import { createVRMAnimationClip, VRMAnimationLoaderPlugin, VRMLookAtQuaternionProxy } from '@pixiv/three-vrm-animation';
import { isEmotion } from './motion.js';
import { LipSyncAnalyzer } from './lip-sync.js?v=3';

export class VrmViewer {
  constructor(canvas, report) {
    this.canvas = canvas;
    this.report = report;
    this.clock = new THREE.Clock();
    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(30, 1, 0.1, 100);
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: false });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.loader = new GLTFLoader();
    this.loader.register((parser) => new VRMLoaderPlugin(parser));
    this.loader.register((parser) => new VRMAnimationLoaderPlugin(parser));
    this.idleClips = [];
    this.emotionClips = new Map();
    this.currentAction = undefined;
    this.currentExpression = 'neutral';
    this.lipSync = new LipSyncAnalyzer();
    this.blinkTimer = 2 + Math.random() * 3;
    this.blinkTime = 0;
    this.frame = this.frame.bind(this);
    this.onResize = this.onResize.bind(this);
  }

  async load(config) {
    if (!config?.vrm_url) throw new Error('VRMファイルが設定されていません。assets/model.vrm を配置してください。');
    this.configureScene(config);
    this.onResize();
    window.addEventListener('resize', this.onResize);
    this.renderer.setAnimationLoop(this.frame);

    let gltf;
    try {
      gltf = await this.loader.loadAsync(config.vrm_url);
    } catch (error) {
      throw new Error(`VRMを読み込めませんでした: ${config.vrm_url}`);
    }
    this.vrm = gltf.userData.vrm;
    if (!this.vrm) throw new Error('指定されたファイルはVRMとして読み込めませんでした。');
    VRMUtils.removeUnnecessaryVertices(this.vrm.scene);
    VRMUtils.combineSkeletons(this.vrm.scene);
    this.vrm.scene.traverse((object) => { object.frustumCulled = false; });
    if (this.vrm.meta?.metaVersion === '0') VRMUtils.rotateVRM0(this.vrm);
    if (this.vrm.lookAt) {
      const proxy = new VRMLookAtQuaternionProxy(this.vrm.lookAt);
      proxy.name = 'lookAtQuaternionProxy';
      this.vrm.scene.add(proxy);
    }
    this.scene.add(this.vrm.scene);
    this.mixer = new THREE.AnimationMixer(this.vrm.scene);
    this.mixer.addEventListener('finished', (event) => this.onAnimationFinished(event));
    await this.loadMotions(config);
    this.setEmotion('neutral');
    this.resumeIdle();
  }

  configureScene(config) {
    const camera = config.camera || {};
    this.camera.fov = camera.fov || 30;
    this.camera.position.fromArray(camera.position || [0, 1.35, 3]);
    this.camera.lookAt(...(camera.target || [0, 1.25, 0]));
    this.camera.updateProjectionMatrix();
    this.renderer.setClearColor(config.background_color || '#202632');
    const light = config.light || {};
    const directional = new THREE.DirectionalLight(light.color || '#ffffff', light.intensity ?? 2.2);
    directional.position.fromArray(light.position || [1, 2, 3]);
    this.scene.add(directional);
    this.scene.add(new THREE.AmbientLight(light.color || '#ffffff', light.ambient_intensity ?? 1));
  }

  async loadMotions(config) {
    const warnings = [];
    for (const url of config.idle_motions || []) {
      try {
        this.idleClips.push(await this.loadMotion(url));
      } catch {
        warnings.push(`待機モーションを読み込めませんでした: ${url}`);
      }
    }
    for (const [emotion, url] of Object.entries(config.emotion_motions || {})) {
      if (!isEmotion(emotion) || !url) continue;
      try {
        this.emotionClips.set(emotion, await this.loadMotion(url));
      } catch {
        warnings.push(`感情モーションを読み込めませんでした: ${url}`);
      }
    }
    if (warnings.length) this.report(warnings.join('\n'));
  }

  async loadMotion(url) {
    const gltf = await this.loader.loadAsync(url);
    const animation = gltf.userData.vrmAnimations?.[0];
    if (!animation) throw new Error('VRMAではありません');
    const clip = createVRMAnimationClip(animation, this.vrm);
    const hips = this.vrm.humanoid?.getNormalizedBoneNode('hips');
    if (hips) {
      clip.tracks = clip.tracks.filter((track) => track.name !== `${hips.name}.position`);
    }
    return clip;
  }

  resumeIdle() {
    if (!this.idleClips.length || !this.mixer) return;
    const next = this.idleClips[Math.floor(Math.random() * this.idleClips.length)];
    this.playClip(next, true);
  }

  playEmotionMotion(emotion) {
    const clip = this.emotionClips.get(emotion);
    if (!clip || !this.mixer) return;
    this.playClip(clip, false);
  }

  playClip(clip, loop) {
    const next = this.mixer.clipAction(clip);
    const previous = this.currentAction;
    if (previous === next && previous.isRunning()) return;
    previous?.fadeOut(0.2);
    next.reset();
    next.setLoop(loop ? THREE.LoopOnce : THREE.LoopOnce, 1);
    next.clampWhenFinished = true;
    next.setEffectiveWeight(1).fadeIn(0.2).play();
    this.currentAction = next;
  }

  onAnimationFinished(event) {
    if (event.action !== this.currentAction) return;
    event.action.fadeOut(0.2);
    this.currentAction = event.action;
    this.resumeIdle();
  }

  setEmotion(emotion) {
    const value = isEmotion(emotion) ? emotion : 'neutral';
    if (!this.vrm?.expressionManager) return;
    this.setExpressionValue(this.currentExpression, 0);
    this.currentExpression = value;
    this.setExpressionValue(value, value === 'neutral' ? 0 : 1);
  }

  setExpressionValue(name, value) {
    const manager = this.vrm?.expressionManager;
    if (!manager) return;
    try {
      const expressionName = manager.expressions
        ?.find((expression) => expression.expressionName?.toLowerCase() === name.toLowerCase())
        ?.expressionName ?? name;
      if (!manager.getExpression || manager.getExpression(expressionName)) {
        manager.setValue(expressionName, value);
      }
    } catch (error) {
      console.warn(`表情 ${name} はこのVRMで使えません`, error);
    }
  }

  startLipSync(analyser) {
    this.lipSync.start(analyser);
  }

  stopLipSync() {
    this.applyMouthWeights(this.lipSync.stop());
  }

  updateLipSync(delta) {
    this.applyMouthWeights(this.lipSync.update(delta));
  }

  applyMouthWeights(weights) {
    for (const [vowel, value] of Object.entries(weights)) {
      this.setExpressionValue(vowel, value);
    }
  }

  updateBlink(delta) {
    this.blinkTimer -= delta;
    if (this.blinkTimer <= 0 && this.blinkTime <= 0) this.blinkTime = 0.16;
    if (this.blinkTime > 0) {
      this.blinkTime -= delta;
      const progress = 1 - Math.max(this.blinkTime, 0) / 0.16;
      this.setExpressionValue('blink', Math.sin(progress * Math.PI));
      if (this.blinkTime <= 0) {
        this.setExpressionValue('blink', 0);
        this.blinkTimer = 2 + Math.random() * 4;
      }
    }
  }

  frame() {
    const delta = Math.min(this.clock.getDelta(), 0.1);
    this.mixer?.update(delta);
    this.updateBlink(delta);
    this.updateLipSync(delta);
    this.vrm?.update(delta);
    this.renderer.render(this.scene, this.camera);
  }

  onResize() {
    const width = this.canvas.clientWidth || window.innerWidth;
    const height = this.canvas.clientHeight || window.innerHeight;
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    this.renderer.setSize(width, height, false);
  }

  dispose() {
    window.removeEventListener('resize', this.onResize);
    this.renderer.setAnimationLoop(null);
    this.mixer?.stopAllAction();
    this.vrm?.scene.traverse((object) => {
      object.geometry?.dispose?.();
      const materials = Array.isArray(object.material) ? object.material : [object.material];
      materials.filter(Boolean).forEach((material) => material.dispose?.());
    });
    this.renderer.dispose();
  }
}
