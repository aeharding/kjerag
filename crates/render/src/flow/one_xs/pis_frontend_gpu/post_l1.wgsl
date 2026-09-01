const L1_ROWS = 540u;
const L1_COLS = 30u;
const L1_PIXELS = 16200u;
const L2_ROWS = 270u;
const L2_COLS = 15u;
const L2_PIXELS = 4050u;
const PUB_ROWS = 1080u;
const PUB_COLS = 60u;
const PUB_PIXELS = 64800u;
const PATCH_ROWS = 178u;
const PATCH_COLS = 8u;
const PATCHES = 1424u;
const HIST_BINS = 159u;
const PATCH_SIZE = 8u;
const PATCH_STRIDE = 3u;
const HINT_FAILURE_TAG = 0x80000000u;

@group(0) @binding(0) var<storage, read> terminal: array<u32>;
@group(0) @binding(1) var<storage, read> images: array<u32>;
@group(0) @binding(3) var<storage, read_write> histogram: array<u32>;
@group(0) @binding(4) var<storage, read_write> fifo: array<u32>;
@group(0) @binding(7) var<storage, read_write> next_hints: array<u32>;
@group(0) @binding(9) var<storage, read_write> filtered: array<u32>;
@group(0) @binding(11) var<storage, read_write> dense_l1: array<u32>;
@group(0) @binding(12) var<storage, read_write> horizontal: array<u32>;
@group(0) @binding(13) var<storage, read_write> public_out: array<u32>;
@group(0) @binding(14) var<storage, read> quantized_values: array<u32>;
@group(0) @binding(16) var<storage, read_write> retained_l2_out: array<u32>;
@group(0) @binding(17) var<storage, read_write> resident_validity: array<atomic<u32>>;

fn raw_at(dir: u32, patch_index: u32, component: u32) -> f32 {
    return bitcast<f32>(terminal[dir * 2u * PATCHES + 2u * patch_index + component]);
}
fn vec_index(dir: u32, pixel: u32, component: u32, pixels: u32) -> u32 {
    return (dir * pixels + pixel) * 2u + component;
}
fn hint_base(dir: u32, level: u32, component: u32) -> u32 {
    if (dir == 0u) {
        if (level == 1u) { return select(0u, 64800u, component == 1u); }
        return select(129600u, 133650u, component == 1u);
    }
    if (level == 1u) { return select(137700u, 202500u, component == 1u); }
    return select(267300u, 271350u, component == 1u);
}
fn finite(v: f32) -> bool { return (bitcast<u32>(v) & 0x7fffffffu) < 0x7f800000u; }
fn nan(v: f32) -> bool { return (bitcast<u32>(v) & 0x7fffffffu) > 0x7f800000u; }
// Rust's selected `as i32` path matches AArch64 FCVTZS: truncate toward zero,
// saturate infinities/overflow, and map NaN to zero.  WGSL's NaN conversion is
// not portable, so spell out that one exceptional result before converting.
fn fcvtzs_bin(scaled: f32) -> i32 {
    if (nan(scaled)) { return 0; }
    return i32(scaled);
}
fn corrected_divide(numerator: f32, denominator: f32) -> f32 {
    let quotient = numerator / denominator;
    let residual = fma(-quotient, denominator, numerator);
    return fma(residual, 1.0 / denominator, quotient);
}

@compute @workgroup_size(64)
fn temporal_median(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= 2u * PATCHES) { return; }
    let dir = id.x / PATCHES;
    let patch_index = id.x % PATCHES;
    let current = raw_at(dir, patch_index, 0u);
    let base = select(bitcast<f32>(0x3f7ffc66u), bitcast<f32>(0xbf7ffc66u), dir == 1u);
    let scale = select(bitcast<f32>(0xc1200000u), bitcast<f32>(0x41200000u), dir == 1u);
    let scaled = (current - base) * scale;
    let converted = fcvtzs_bin(scaled);
    let nan_is_bin_zero = nan(scaled);
    var accepted = nan_is_bin_zero ||
        (finite(scaled) && converted >= 0 && converted < i32(HIST_BINS));
    let bin = select(0u, u32(converted), accepted);
    let fb = id.x * 5u;
    var head = fifo[fb + 3u];
    var len = fifo[fb + 4u];
    let hb = id.x * HIST_BINS;
    if (accepted) {
        if (len == 3u) {
            let oldest = bitcast<f32>(fifo[fb + head]);
            let old_bin = u32(fcvtzs_bin((oldest - base) * scale));
            histogram[hb + old_bin] = histogram[hb + old_bin] - 1u;
            fifo[fb + head] = bitcast<u32>(current);
            head = (head + 1u) % 3u;
        } else {
            fifo[fb + ((head + len) % 3u)] = bitcast<u32>(current);
            len += 1u;
        }
        histogram[hb + bin] += 1u;
        fifo[fb + 3u] = head;
        fifo[fb + 4u] = len;
        let rank = (len + 1u) / 2u;
        var cumulative = 0u;
        var median = 0u;
        for (var b = 0u; b < HIST_BINS; b += 1u) {
            cumulative += histogram[hb + b];
            if (cumulative >= rank) { median = b; break; }
        }
        filtered[2u * id.x] = quantized_values[dir * HIST_BINS + median];
    } else {
        filtered[2u * id.x] = bitcast<u32>(current);
    }
    filtered[2u * id.x + 1u] = bitcast<u32>(raw_at(dir, patch_index, 1u));
}

fn image_at(dir: u32, use_target: bool, row: u32, col: u32) -> f32 {
    let physical = select(dir, 1u - dir, use_target);
    return f32(images[physical * L1_PIXELS + row * L1_COLS + col]);
}
fn ordered_clamp(v0: f32, upper: f32) -> f32 {
    var v = v0; if (v < 0.0) { v = 0.0; } if (upper < v) { v = upper; } return v;
}
fn sample_index(v: f32) -> u32 {
    if (nan(v)) { return 0u; }
    return u32(v);
}
fn sample_target(dir: u32, row0: f32, col0: f32) -> f32 {
    let row = ordered_clamp(row0, bitcast<f32>(0x4406bff0u));
    let col = ordered_clamp(col0, bitcast<f32>(0x41e7fdf4u));
    let r0 = sample_index(row); let c0 = sample_index(col); let r1 = r0 + 1u; let c1 = c0 + 1u;
    let rf = row - f32(r0); let cf = col - f32(c0);
    let ri = f32(r1) - row; let ci = f32(c1) - col;
    let br = image_at(dir,true,r1,c1); let bl = image_at(dir,true,r1,c0);
    let tr = image_at(dir,true,r0,c1); let tl = image_at(dir,true,r0,c0);
    let bottom = fma(cf * rf, br, (ci * rf) * bl);
    let right = fma(cf * ri, tr, bottom);
    return fma(ci * ri, tl, right);
}
fn first_patch(pixel: u32, count: u32) -> u32 {
    let start = select(0u, pixel + 1u - PATCH_SIZE, pixel + 1u >= PATCH_SIZE);
    return min((start + PATCH_STRIDE - 1u) / PATCH_STRIDE, count - 1u);
}
fn vote(dir: u32, row: u32, col: u32, raw: bool) -> vec2<f32> {
    let first_r = first_patch(row, PATCH_ROWS); let last_r = min(row/PATCH_STRIDE,PATCH_ROWS-1u);
    let first_c = first_patch(col, PATCH_COLS); let last_c = min(col/PATCH_STRIDE,PATCH_COLS-1u);
    let source = image_at(dir,false,row,col);
    var sum = vec2<f32>(0.0); var weight_sum = 0.0;
    for (var pr=first_r; pr<=last_r; pr+=1u) { for (var pc=first_c; pc<=last_c; pc+=1u) {
        let p=pr*PATCH_COLS+pc;
        let flow = select(vec2<f32>(bitcast<f32>(filtered[2u*(dir*PATCHES+p)]),bitcast<f32>(filtered[2u*(dir*PATCHES+p)+1u])), vec2<f32>(raw_at(dir,p,0u),raw_at(dir,p,1u)), raw);
        let difference=abs(source-sample_target(dir,f32(row)+flow.y,f32(col)+flow.x));
        let weight=select(1.0,corrected_divide(1.0,difference),difference>1.0);
        sum.x=fma(weight,flow.x,sum.x); sum.y=fma(weight,flow.y,sum.y); weight_sum+=weight;
    }}
    return vec2<f32>(corrected_divide(sum.x,weight_sum), corrected_divide(sum.y,weight_sum));
}
fn hint_enabled(row:u32,col:u32)->bool {
    if (row>=4u && col>=4u && (row-4u)%3u==0u && (col-4u)%3u==0u && (row-4u)/3u<PATCH_ROWS && (col-4u)/3u<PATCH_COLS) { return true; }
    for(var dr=0u;dr<2u;dr+=1u){for(var dc=0u;dc<2u;dc+=1u){
        if(row>=8u+dr && col>=8u+dc && (row-8u-dr)%6u==0u && (col-8u-dc)%6u==0u &&
           (row-8u-dr)/6u<88u && (col-8u-dc)/6u<3u){return true;}
    }} return false;
}

@compute @workgroup_size(64)
fn make_hint_l1(@builtin(global_invocation_id) id:vec3<u32>){
    if(id.x>=2u*L1_PIXELS){return;} let dir=id.x/L1_PIXELS; let pixel=id.x%L1_PIXELS;
    let row=pixel/L1_COLS; let col=pixel%L1_COLS; if(!hint_enabled(row,col)){return;}
    let v=vote(dir,row,col,true);
    if(!finite(v.x)){atomicMin(&resident_validity[0],HINT_FAILURE_TAG|(dir<<15u)|pixel);return;}
    if(!finite(v.y)){atomicMin(&resident_validity[0],HINT_FAILURE_TAG|(dir<<15u)|(1u<<14u)|pixel);return;}
    next_hints[hint_base(dir,1u,0u)+pixel]=bitcast<u32>(v.x);
    next_hints[hint_base(dir,1u,1u)+pixel]=bitcast<u32>(v.y);
}
@compute @workgroup_size(64)
fn make_hint_l2(@builtin(global_invocation_id) id:vec3<u32>){
    if(id.x>=2u*L2_PIXELS){return;} let dir=id.x/L2_PIXELS; let p=id.x%L2_PIXELS;
    let row=p/L2_COLS; let col=p%L2_COLS;
    if(row<4u||col<4u||(row-4u)%3u!=0u||(col-4u)%3u!=0u||(row-4u)/3u>=88u||(col-4u)/3u>=3u){return;}
    let sr=min(2u*row,L1_ROWS-1u); let sc=min(2u*col,L1_COLS-1u);
    for(var component=0u;component<2u;component+=1u){
        let l1=hint_base(dir,1u,component);
        let x00=bitcast<f32>(next_hints[l1+sr*L1_COLS+sc]);
        let x01=bitcast<f32>(next_hints[l1+sr*L1_COLS+min(sc+1u,L1_COLS-1u)]);
        let x10=bitcast<f32>(next_hints[l1+min(sr+1u,L1_ROWS-1u)*L1_COLS+sc]);
        let x11=bitcast<f32>(next_hints[l1+min(sr+1u,L1_ROWS-1u)*L1_COLS+min(sc+1u,L1_COLS-1u)]);
        let sum0=x00+0.0; let sum1=sum0+x01; let sum2=sum1+x10; let sum3=sum2+x11;
        let propagated=(sum3*0.25)*0.5;
        if(!finite(propagated)){atomicMin(&resident_validity[0],HINT_FAILURE_TAG|(1u<<16u)|(dir<<15u)|(component<<14u)|p);return;}
        next_hints[hint_base(dir,2u,component)+p]=bitcast<u32>(propagated);
    }
}

@compute @workgroup_size(64)
fn densify_cold(@builtin(global_invocation_id) id:vec3<u32>){
    if(id.x>=2u*L1_PIXELS){return;}
    let dir=id.x/L1_PIXELS; let pixel=id.x%L1_PIXELS;
    let row=pixel/L1_COLS; let col=pixel%L1_COLS;
    let fresh=vote(dir,row,col,false);
    dense_l1[vec_index(dir,pixel,0u,L1_PIXELS)]=bitcast<u32>(fresh.x);
    dense_l1[vec_index(dir,pixel,1u,L1_PIXELS)]=bitcast<u32>(fresh.y);
}

fn dense(dir:u32,row:u32,col:u32,component:u32)->f32{return bitcast<f32>(dense_l1[vec_index(dir,row*L1_COLS+col,component,L1_PIXELS)]);}
@compute @workgroup_size(64)
fn resize_horizontal(@builtin(global_invocation_id) id:vec3<u32>){
    let total=2u*L1_ROWS*PUB_COLS*2u;if(id.x>=total){return;}let dir=id.x/(L1_ROWS*PUB_COLS*2u);let local=id.x%(L1_ROWS*PUB_COLS*2u);let row=local/(PUB_COLS*2u);let lane=local%(PUB_COLS*2u);let col=lane/2u;let component=lane%2u;
    var left=0u;var right=1u;var lw=1.0;var rw=0.0;if(col==PUB_COLS-1u){left=L1_COLS-1u;right=left;}else if(col>0u&&col%2u==0u){left=col/2u-1u;right=col/2u;lw=0.25;rw=0.75;}else if(col>0u){left=col/2u;right=left+1u;lw=0.75;rw=0.25;}
    let a=dense(dir,row,left,component);let b=dense(dir,row,right,component);if(col==PUB_COLS-1u){horizontal[2u*id.x]=bitcast<u32>(a);horizontal[2u*id.x+1u]=bitcast<u32>(0.0);}else if(lane<116u){horizontal[2u*id.x]=bitcast<u32>(a*lw);horizontal[2u*id.x+1u]=bitcast<u32>(b*rw);}else{horizontal[2u*id.x]=bitcast<u32>(a);horizontal[2u*id.x+1u]=bitcast<u32>(b*rw);}
}
@compute @workgroup_size(64)
fn resize_vertical(@builtin(global_invocation_id) id:vec3<u32>){
    let total=2u*PUB_ROWS*PUB_COLS*2u;if(id.x>=total){return;}let dir=id.x/(PUB_ROWS*PUB_COLS*2u);let local=id.x%(PUB_ROWS*PUB_COLS*2u);let row=local/(PUB_COLS*2u);let lane=local%(PUB_COLS*2u);
    var top=0u;var bottom=0u;var tw=0.25;var bw=0.75;if(row==PUB_ROWS-1u){top=L1_ROWS-1u;bottom=top;tw=0.75;bw=0.25;}else if(row>0u&&row%2u==0u){top=row/2u-1u;bottom=row/2u;tw=0.25;bw=0.75;}else if(row>0u){top=row/2u;bottom=top+1u;tw=0.75;bw=0.25;}
    let base=dir*L1_ROWS*PUB_COLS*2u;let top_at=base+top*PUB_COLS*2u+lane;let bottom_at=base+bottom*PUB_COLS*2u+lane;let top_left=bitcast<f32>(horizontal[2u*top_at]);let top_right=bitcast<f32>(horizontal[2u*top_at+1u]);let bottom_left=bitcast<f32>(horizontal[2u*bottom_at]);let bottom_right=bitcast<f32>(horizontal[2u*bottom_at+1u]);let scalar=lane>=116u&&lane<118u;let copy=lane>=118u;var a=select(top_left+top_right,fma(top_left,select(0.25,0.75,(lane/2u)%2u==1u),top_right),scalar);var b=select(bottom_left+bottom_right,fma(bottom_left,select(0.25,0.75,(lane/2u)%2u==1u),bottom_right),scalar);if(copy){a=top_left;b=bottom_left;}let lower=b*bw;let value=fma(a,tw,lower);public_out[id.x]=bitcast<u32>(value*2.0);
}
@compute @workgroup_size(64)
fn repair_periodic(@builtin(global_invocation_id) id:vec3<u32>){
    let total=2u*5u*PUB_COLS*2u;if(id.x>=total){return;}let dir=id.x/(5u*PUB_COLS*2u);let local=id.x%(5u*PUB_COLS*2u);let pair=local/(PUB_COLS*2u);let lane=local%(PUB_COLS*2u);let row=51u+pair;let partner=1022u+pair;
    var w=bitcast<f32>(0x3d3fb800u);if(pair==1u){w=bitcast<f32>(0x3e0de200u);}else if(pair==2u){w=bitcast<f32>(0x3ea5d800u);}else if(pair==3u){w=bitcast<f32>(0x3f025f80u);}else if(pair==4u){w=bitcast<f32>(0x3f31d300u);}
    let base=dir*PUB_ROWS*PUB_COLS*2u;let a=base+row*PUB_COLS*2u+lane;let b=base+partner*PUB_COLS*2u+lane;let partner_term=bitcast<f32>(public_out[b])*(1.0-w);let result=fma(bitcast<f32>(public_out[a]),w,partner_term);public_out[a]=bitcast<u32>(result);public_out[b]=bitcast<u32>(result);
}

@compute @workgroup_size(64)
fn make_retained_l2(@builtin(global_invocation_id) id:vec3<u32>){
    if(id.x>=2u*L2_PIXELS){return;}
    let dir=id.x/L2_PIXELS; let pixel=id.x%L2_PIXELS;
    let row=pixel/L2_COLS; let col=pixel%L2_COLS;
    let top=4u*row+1u; let bottom=top+1u; let left=4u*col+1u;
    for(var component=0u;component<2u;component+=1u){
        let x00=bitcast<f32>(public_out[vec_index(dir,top*PUB_COLS+left,component,PUB_PIXELS)]);
        let x01=bitcast<f32>(public_out[vec_index(dir,top*PUB_COLS+left+1u,component,PUB_PIXELS)]);
        let x10=bitcast<f32>(public_out[vec_index(dir,bottom*PUB_COLS+left,component,PUB_PIXELS)]);
        let x11=bitcast<f32>(public_out[vec_index(dir,bottom*PUB_COLS+left+1u,component,PUB_PIXELS)]);
        let top_half=(x00+x01)*0.5;
        let bottom_half=(x10+x11)*0.5;
        let resized=(top_half+bottom_half)*0.5;
        retained_l2_out[vec_index(dir,pixel,component,L2_PIXELS)]=bitcast<u32>(resized*0.25);
    }
}
