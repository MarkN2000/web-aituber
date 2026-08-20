export const EMOTIONS = ['neutral', 'happy', 'sad', 'angry', 'surprised'];

export function isEmotion(value) {
  return EMOTIONS.includes(value);
}
