import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { VRMLoaderPlugin, VRMUtils } from '@pixiv/three-vrm';
import { createVRMAnimationClip, VRMAnimationLoaderPlugin, VRMLookAtQuaternionProxy } from '@pixiv/three-vrm-animation';
import { motionFileName } from './debug.js?v=1';
import { isEmotion } from './motion.js';
import { LipSyncAnalyzer } from './lip-sync.js?v=3';

export class VrmViewer {
  constructor(canvas, report, { showFoodPropGizmo = false, onDebugStateChange } = {}) {
    this.canvas = canvas;
    this.report = report;
    this.clock = new THREE.Clock();
    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(30, 1, 0.1, 100);
    this.renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.loader = new GLTFLoader();
    this.loader.register((parser) => new VRMLoaderPlugin(parser));
    this.loader.register((parser) => new VRMAnimationLoaderPlugin(parser));
    this.textureLoader = new THREE.TextureLoader();
    this.idleClips = [];
    this.emotionClips = new Map();
    this.currentAction = undefined;
    this.currentMotion = undefined;
    this.currentExpression = 'neutral';
    this.lipSync = new LipSyncAnalyzer();
    this.blinkTimer = 2 + Math.random() * 3;
    this.blinkTime = 0;
    this.foodActionId = 0;
    this.showFoodPropGizmo = showFoodPropGizmo;
    this.onDebugStateChange = onDebugStateChange;
    this.lastDebugStateKey = undefined;
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
    this.configureFoodProp(config.food_prop);
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
    this.renderer.setClearColor(0x000000, 0);
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

  configureFoodProp(config = {}) {
    const hand = this.vrm.humanoid?.getRawBoneNode('rightHand');
    if (!hand) {
      this.report('食事用Quadを配置できませんでした: VRMの右手が見つかりません。');
      return;
    }

    const position = config.position || [0, 0, 0];
    const rotation = config.rotation_degrees || [0, 0, 0];
    this.foodPropSize = Number(config.size) > 0 ? Number(config.size) : 0.2;
    this.foodAnchor = new THREE.Object3D();
    this.foodAnchor.name = 'FoodPropAnchor';
    this.foodAnchor.position.fromArray(position);
    this.foodAnchor.rotation.set(...rotation.map(THREE.MathUtils.degToRad));
    hand.add(this.foodAnchor);
    this.createFoodPropGizmo();
  }

  createFoodPropGizmo() {
    if (!this.showFoodPropGizmo || !this.foodAnchor) return;

    const gizmo = new THREE.Group();
    gizmo.name = 'FoodPropDebugGizmo';
    const axes = new THREE.AxesHelper(Math.max(this.foodPropSize * 0.75, 0.05));
    axes.renderOrder = 1000;
    for (const material of Array.isArray(axes.material) ? axes.material : [axes.material]) {
      material.depthTest = false;
    }
    const plane = new THREE.PlaneGeometry(this.foodPropSize, this.foodPropSize);
    const frameGeometry = new THREE.EdgesGeometry(plane);
    plane.dispose();
    const frame = new THREE.LineSegments(
      frameGeometry,
      new THREE.LineBasicMaterial({ color: 0xffff00, depthTest: false, toneMapped: false }),
    );
    frame.renderOrder = 1000;
    gizmo.add(axes, frame);
    this.foodAnchor.add(gizmo);
    this.foodPropGizmo = gizmo;
  }

  disposeFoodPropGizmo() {
    if (!this.foodPropGizmo) return;
    this.foodAnchor?.remove(this.foodPropGizmo);
    this.foodPropGizmo.traverse((object) => {
      object.geometry?.dispose?.();
      const materials = Array.isArray(object.material) ? object.material : [object.material];
      materials.filter(Boolean).forEach((material) => material.dispose?.());
    });
    this.foodPropGizmo = undefined;
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
    return { clip, fileName: motionFileName(url) };
  }

  resumeIdle() {
    if (!this.idleClips.length || !this.mixer) {
      this.currentAction = undefined;
      this.currentMotion = undefined;
      this.reportDebugState();
      return;
    }
    const motion = this.idleClips[Math.floor(Math.random() * this.idleClips.length)];
    this.playClip(motion, true, 'idle');
  }

  playEmotionMotion(emotion) {
    const motion = this.emotionClips.get(emotion);
    if (!motion || !this.mixer) return;
    this.playClip(motion, false, 'emotion');
  }

  playClip(motion, loop, kind) {
    const next = this.mixer.clipAction(motion.clip);
    const previous = this.currentAction;
    if (previous === next && previous.isRunning()) return;
    previous?.fadeOut(0.2);
    next.reset();
    next.setLoop(loop ? THREE.LoopOnce : THREE.LoopOnce, 1);
    next.clampWhenFinished = true;
    next.setEffectiveWeight(1).fadeIn(0.2).play();
    this.currentAction = next;
    this.currentMotion = { fileName: motion.fileName, kind };
    this.reportDebugState();
  }

  onAnimationFinished(event) {
    if (event.action !== this.currentAction) return;
    event.action.fadeOut(0.2);
    this.currentAction = undefined;
    this.currentMotion = undefined;
    this.resumeIdle();
  }

  setEmotion(emotion) {
    const value = isEmotion(emotion) ? emotion : 'neutral';
    if (this.vrm?.expressionManager) this.setExpressionValue(this.currentExpression, 0);
    this.currentExpression = value;
    if (this.vrm?.expressionManager) this.setExpressionValue(value, value === 'neutral' ? 0 : 1);
    this.reportDebugState();
  }

  reportDebugState() {
    if (!this.onDebugStateChange) return;
    const state = {
      motionFileName: this.currentMotion?.fileName,
      motionKind: this.currentMotion?.kind,
      expression: this.currentExpression,
    };
    const stateKey = JSON.stringify(state);
    if (stateKey === this.lastDebugStateKey) return;
    this.lastDebugStateKey = stateKey;
    this.onDebugStateChange(state);
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

  playFoodAction(imageUrl, consumeAtMs, durationMs) {
    this.clearFoodProp();
    if (!this.foodAnchor) {
      this.report('食事用Quadを表示できませんでした: 右手の配置設定を確認してください。');
      return;
    }

    const duration = Math.max(Number(durationMs) || 0, 1);
    const consumeAt = Math.min(Math.max(Number(consumeAtMs) || 0, 0), duration);
    const actionId = this.foodActionId;
    this.foodAction = {
      id: actionId,
      startedAt: performance.now(),
      consumeAt,
      duration,
    };

    this.textureLoader.loadAsync(imageUrl)
      .then((texture) => {
        if (this.foodAction?.id !== actionId) {
          texture.dispose();
          return;
        }
        texture.colorSpace = THREE.SRGBColorSpace;
        const geometry = new THREE.PlaneGeometry(this.foodPropSize, this.foodPropSize);
        const material = new THREE.MeshBasicMaterial({
          map: texture,
          side: THREE.DoubleSide,
          alphaTest: 0.5,
          toneMapped: false,
        });
        const mesh = new THREE.Mesh(geometry, material);
        mesh.name = 'FoodPropQuad';
        this.foodAnchor.add(mesh);
        this.foodMesh = mesh;
      })
      .catch((error) => {
        console.error('食事用画像を読み込めませんでした', error);
        if (this.foodAction?.id === actionId) {
          this.report('食事用の絵を表示できませんでした。');
        }
      });
  }

  updateFoodAction() {
    const action = this.foodAction;
    if (!action) return;
    const elapsed = performance.now() - action.startedAt;
    if (this.foodMesh && elapsed >= action.consumeAt) {
      const consumeDuration = Math.max(action.duration - action.consumeAt, 1);
      const progress = THREE.MathUtils.clamp((elapsed - action.consumeAt) / consumeDuration, 0, 1);
      const remaining = 1 - progress;
      this.foodMesh.scale.setScalar(remaining);
    }
    if (elapsed >= action.duration) this.clearFoodProp();
  }

  clearFoodProp() {
    this.foodActionId += 1;
    this.foodAction = undefined;
    if (!this.foodMesh) return;
    this.foodAnchor?.remove(this.foodMesh);
    this.foodMesh.geometry.dispose();
    this.foodMesh.material.map?.dispose();
    this.foodMesh.material.dispose();
    this.foodMesh = undefined;
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
    this.updateFoodAction();
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
    this.clearFoodProp();
    this.disposeFoodPropGizmo();
    this.mixer?.stopAllAction();
    this.vrm?.scene.traverse((object) => {
      object.geometry?.dispose?.();
      const materials = Array.isArray(object.material) ? object.material : [object.material];
      materials.filter(Boolean).forEach((material) => material.dispose?.());
    });
    this.renderer.dispose();
  }
}
