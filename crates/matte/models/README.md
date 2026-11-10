# Bundled model

`selfie_segmentation.tflite` is MediaPipe's Selfie Segmentation model (the general, 256 × 256
one), exactly as Google publishes it: 249,505 bytes, SHA-256
`9ee168ec7c8f2a16c56fe8e1cfbc514974cbbb7e434051b455635f1bd1462f5c`. It was taken from the
`mediapipe` 0.10.14 package on PyPI, where it is
`mediapipe/modules/selfie_segmentation/selfie_segmentation.tflite`.

MediaPipe is Copyright The MediaPipe Authors and licensed under the Apache License, Version 2.0
(the text is this repository's `LICENSE-APACHE`); its home is <https://github.com/google/mediapipe>.
The file is not modified. Google's model card for it describes what it was trained on and what it
is for: one or a few people close to the camera, as in a video call or a selfie.

What it takes and gives:

- in: one picture, 256 × 256, red, green and blue as on screen from 0 to 1 (the picture is
  stretched to that size, not padded);
- out: 256 × 256 values from 0 to 1, how likely each pixel is part of a person.

Its weights are half-precision floats unpacked by `DEQUANTIZE` steps, and it uses eleven kinds of
step: `CONV_2D`, `DEPTHWISE_CONV_2D`, `AVERAGE_POOL_2D` (over the whole picture), `HARD_SWISH`,
`RELU`, `LOGISTIC`, `ADD`, `MUL`, `RESIZE_BILINEAR`, and MediaPipe's own
`Convolution2DTransposeBias`. `src/tflite.rs` reads exactly those and refuses a file with anything
else, so another model is not a drop-in replacement.

`tests/astronaut.png` is scikit-image's `astronaut` test picture at the model's size: NASA's
portrait of Eileen Collins, in the public domain. `tests/astronaut_person.png` is what TensorFlow
Lite's own interpreter (`ai-edge-litert`) answers for it with this model, kept to 8 bits; the
crate's tests hold its answer to that.
