/** Small helpers for compiling programs and wiring instanced attributes. */

/** Compiles and links one program; null (with the log in the console) on failure. */
export function createProgram(gl: WebGL2RenderingContext, vertex: string, fragment: string): WebGLProgram | null {
  const program = gl.createProgram();
  const shaders: WebGLShader[] = [];
  for (const [type, source] of [
    [gl.VERTEX_SHADER, vertex],
    [gl.FRAGMENT_SHADER, fragment],
  ] as const) {
    const shader = gl.createShader(type);
    if (shader === null) {
      return null;
    }
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      console.error('orb shader failed to compile', gl.getShaderInfoLog(shader));
      return null;
    }
    gl.attachShader(program, shader);
    shaders.push(shader);
  }
  gl.linkProgram(program);
  for (const shader of shaders) {
    gl.deleteShader(shader);
  }
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    console.error('orb program failed to link', gl.getProgramInfoLog(program));
    return null;
  }
  return program;
}

/** One attribute in an interleaved float buffer. */
export interface AttributeSpec {
  readonly name: string;
  readonly size: number;
}

/** A static float buffer with its interleaved attributes. */
export interface BufferSpec {
  readonly data: Float32Array;
  readonly attributes: readonly AttributeSpec[];
  /** True for one record per instance, false for one per vertex. */
  readonly perInstance: boolean;
}

/** Uploads each buffer once and records its attributes in a new vertex array. */
export function createVertexArray(
  gl: WebGL2RenderingContext,
  program: WebGLProgram,
  buffers: readonly BufferSpec[],
): { vao: WebGLVertexArrayObject; buffers: WebGLBuffer[] } {
  const vao = gl.createVertexArray();
  const created: WebGLBuffer[] = [];
  gl.bindVertexArray(vao);
  for (const spec of buffers) {
    const buffer = gl.createBuffer();
    created.push(buffer);
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, spec.data, gl.STATIC_DRAW);
    const stride = spec.attributes.reduce((sum, attribute) => sum + attribute.size, 0) * 4;
    let offset = 0;
    for (const attribute of spec.attributes) {
      const location = gl.getAttribLocation(program, attribute.name);
      if (location >= 0) {
        gl.enableVertexAttribArray(location);
        gl.vertexAttribPointer(location, attribute.size, gl.FLOAT, false, stride, offset);
        gl.vertexAttribDivisor(location, spec.perInstance ? 1 : 0);
      }
      offset += attribute.size * 4;
    }
  }
  gl.bindVertexArray(null);
  return { vao, buffers: created };
}

/** Every active uniform's location by name (array uniforms under their base name). */
export function uniformLocations(gl: WebGL2RenderingContext, program: WebGLProgram): Map<string, WebGLUniformLocation> {
  const locations = new Map<string, WebGLUniformLocation>();
  const count = gl.getProgramParameter(program, gl.ACTIVE_UNIFORMS) as number;
  for (let index = 0; index < count; index += 1) {
    const info = gl.getActiveUniform(program, index);
    if (info === null) {
      continue;
    }
    const name = info.name.replace(/\[0\]$/, '');
    const location = gl.getUniformLocation(program, info.name);
    if (location !== null) {
      locations.set(name, location);
    }
  }
  return locations;
}
