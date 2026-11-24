// One fused pull-stream + collide step of the D3Q19 solver, the same
// arithmetic as Lbm::advance on the CPU. One thread per cell.
// Populations are stored direction-major (q * n + cell) so neighbouring
// threads read neighbouring addresses.

struct Params {
    nx: u32, ny: u32, nz: u32, n: u32,
    gx: u32, bits: u32, step: u32, pad: u32,
    tau0: f32, cs2: f32, u_ramp: f32, ground_u: f32,
};

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> fs: array<f32>;
@group(0) @binding(2) var<storage, read_write> fd: array<f32>;
@group(0) @binding(3) var<storage, read> flags: array<u32>;
@group(0) @binding(4) var<storage, read> widx: array<u32>;
@group(0) @binding(5) var<storage, read> wdelta: array<f32>;
@group(0) @binding(6) var<storage, read> wallu: array<f32>;
@group(0) @binding(7) var<storage, read_write> force: array<f32>;
@group(0) @binding(8) var<storage, read_write> stats: array<atomic<u32>>;

const FLUID: u32 = 0u;
const SOLID: u32 = 1u;
const GROUND: u32 = 2u;
const FARFIELD: u32 = 3u;
const OUTLET: u32 = 4u;
const NONE: u32 = 0xffffffffu;

//TABLES

fn at(cell: u32, q: u32) -> u32 {
    return q * P.n + cell;
}

fn offset(q: u32) -> i32 {
    return CX[q] + i32(P.nx) * (CY[q] + i32(P.ny) * CZ[q]);
}

fn feq(q: u32, rho: f32, u: vec3<f32>) -> f32 {
    let c = vec3<f32>(f32(CX[q]), f32(CY[q]), f32(CZ[q]));
    let cu = dot(c, u);
    return WQ[q] * rho * (1.0 + 3.0 * cu + 4.5 * cu * cu - 1.5 * dot(u, u));
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let cell = id.x + id.y * P.gx * 64u;
    if (cell >= P.n) {
        return;
    }
    let flag = flags[cell];
    if (flag == FARFIELD) {
        for (var q = 0u; q < 19u; q++) {
            fd[at(cell, q)] = feq(q, 1.0, vec3<f32>(P.u_ramp, 0.0, 0.0));
        }
        return;
    }
    if (flag == OUTLET) {
        let nb = cell - 1u;
        var rho = 0.0;
        var m = vec3<f32>(0.0);
        for (var q = 0u; q < 19u; q++) {
            let v = fs[at(nb, q)];
            rho += v;
            m += v * vec3<f32>(f32(CX[q]), f32(CY[q]), f32(CZ[q]));
        }
        let u = m / rho;
        for (var q = 0u; q < 19u; q++) {
            fd[at(cell, q)] = feq(q, 1.0, u);
        }
        return;
    }
    if (flag != FLUID) {
        return;
    }
    let slip = (P.bits & 1u) != 0u;
    let moving = (P.bits & 2u) != 0u;
    let i = f32(cell % P.nx);
    let wi = widx[cell];
    var f: array<f32, 19>;
    var force_acc = vec4<f32>(0.0);
    for (var q = 0u; q < 19u; q++) {
        let o = offset(q);
        let src = u32(i32(cell) - o);
        let sf = flags[src];
        if (slip && sf == GROUND && CZ[q] == 1) {
            let along = u32(i32(cell) - CX[q] - i32(P.nx) * CY[q]);
            let fa = flags[along];
            if (fa == SOLID || fa == GROUND) {
                f[q] = fs[at(cell, OPP[q])];
            } else {
                f[q] = fs[at(along, MZ[q])];
            }
            continue;
        }
        if (sf == SOLID || sf == GROUND) {
            let qb = OPP[q];
            let post = fs[at(cell, qb)];
            let c = vec3<f32>(f32(CX[q]), f32(CY[q]), f32(CZ[q]));
            var uw = vec3<f32>(0.0);
            if (sf == GROUND) {
                uw = vec3<f32>(P.ground_u, 0.0, 0.0);
            } else if (moving) {
                uw = P.u_ramp * vec3<f32>(wallu[src * 3u], wallu[src * 3u + 1u], wallu[src * 3u + 2u]);
            }
            let wt = 6.0 * WQ[q] * dot(c, uw);
            var val = post + wt;
            if (sf == SOLID && wi != NONE) {
                let d = wdelta[wi * 19u + q];
                if (d < 0.5) {
                    let nxt = u32(i32(cell) + o);
                    if (flags[nxt] == FLUID) {
                        val = 2.0 * d * post + (1.0 - 2.0 * d) * fs[at(nxt, qb)] + wt;
                    }
                } else {
                    let k = 0.5 / d;
                    val = k * post + (1.0 - k) * fs[at(cell, q)] + k * wt;
                }
            }
            f[q] = val;
            if (sf == SOLID) {
                let s = post + val - 2.0 * WQ[q];
                let dd = post - val;
                let fq = -c * s - uw * dd;
                force_acc += vec4<f32>(fq, fq.z * (i - 0.5 * c.x));
            }
        } else {
            f[q] = fs[at(src, q)];
        }
    }
    if (wi != NONE) {
        force[wi * 4u] = force_acc.x;
        force[wi * 4u + 1u] = force_acc.y;
        force[wi * 4u + 2u] = force_acc.z;
        force[wi * 4u + 3u] = force_acc.w;
    }
    var rho = 0.0;
    var m = vec3<f32>(0.0);
    for (var q = 0u; q < 19u; q++) {
        rho += f[q];
        m += f[q] * vec3<f32>(f32(CX[q]), f32(CY[q]), f32(CZ[q]));
    }
    if (!(rho > 0.0 && rho < 1.0e30)) {
        atomicStore(&stats[1], 1u);
        for (var q = 0u; q < 19u; q++) {
            fd[at(cell, q)] = f[q];
        }
        return;
    }
    let u = m / rho;
    atomicMax(&stats[0], bitcast<u32>(dot(u, u)));
    var eq: array<f32, 19>;
    var p0 = 0.0; var p1 = 0.0; var p2 = 0.0; var p3 = 0.0; var p4 = 0.0; var p5 = 0.0;
    for (var q = 0u; q < 19u; q++) {
        eq[q] = feq(q, rho, u);
        let d = f[q] - eq[q];
        let cx = f32(CX[q]); let cy = f32(CY[q]); let cz = f32(CZ[q]);
        p0 += cx * cx * d; p1 += cy * cy * d; p2 += cz * cz * d;
        p3 += cx * cy * d; p4 += cx * cz * d; p5 += cy * cz * d;
    }
    var tau = P.tau0;
    if (P.cs2 > 0.0) {
        let qn = sqrt(p0 * p0 + p1 * p1 + p2 * p2 + 2.0 * (p3 * p3 + p4 * p4 + p5 * p5));
        tau = 0.5 * (P.tau0 + sqrt(P.tau0 * P.tau0 + 18.0 * 1.4142135 * P.cs2 * qn / rho));
    }
    let keep = 1.0 - 1.0 / tau;
    let tr = (p0 + p1 + p2) / 3.0;
    for (var q = 0u; q < 19u; q++) {
        let cx = f32(CX[q]); let cy = f32(CY[q]); let cz = f32(CZ[q]);
        let neq = 4.5 * WQ[q] * (cx * cx * p0 + cy * cy * p1 + cz * cz * p2
            + 2.0 * (cx * cy * p3 + cx * cz * p4 + cy * cz * p5) - tr);
        fd[at(cell, q)] = eq[q] + keep * neq;
    }
}
