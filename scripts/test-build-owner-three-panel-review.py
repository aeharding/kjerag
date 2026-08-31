#!/usr/bin/env python3
import hashlib, importlib.util, json, os, struct, tempfile, unittest
from contextlib import ExitStack
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("owner_three_panel", HERE / "build-owner-three-panel-review.py")
M = importlib.util.module_from_spec(SPEC); SPEC.loader.exec_module(M)
COMMIT, TREE, PLAYBACK_SHA, TRACE_SHA = "a"*40, "b"*40, "c"*64, "d"*64

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def png(path, payload=b""): path.write_bytes(M.PNG_SIGNATURE+struct.pack(">I",13)+b"IHDR"+struct.pack(">II",M.WIDTH,M.HEIGHT)+payload)
def leaf(path,key="file"): return {key:path.name,"bytes":path.stat().st_size,"sha256":sha(path)}
def ident(n): return {"device":1,"inode":n,"mode":33261,"links":1,"uid":1000,"mtime_seconds":1,"mtime_nanoseconds":2,"ctime_seconds":3,"ctime_nanoseconds":4}
def build(exe_sha):
    return {"package_version":"0.2.0","embedded_git_commit":COMMIT,"embedded_git_tree":TREE,
            "embedded_git_dirty":"false","dirty_at_build":False,"runtime_git_commit":COMMIT,
            "runtime_git_tree":TREE,"runtime_tracked_tree_clean":True,"cargo_lock":{"sha256":"e"*64},
            "executable":{"file":"/proc/self/exe","bytes":10,"sha256":exe_sha,"stable_identity":ident(9)}}

def fixtures(root):
    kd,sd,td=root/"kjerag",root/"studio",root/"trace"
    for p in (kd,sd,td): p.mkdir()
    source=[]
    for n,c in enumerate(M.OWNER_SOURCES):
        source.append({"path":f"/source/{c['basename']}",**{k:c[k] for k in ("bytes","sha256","decoder_lane","picked")},"stable_identity":ident(n+1)})
    request={"start":M.START,"count":M.COUNT,"end_inclusive":M.END,"no_seek":True,"cold_start_at_range_boundary":False,"every_source_frame_consumed":True,"captured_map_substitution":False}
    run={"presented":M.END+1,"dropped":0,"starved":0,"scene_redraws":M.END+1}
    kf,sf,tf=[],[],[]
    for i in range(M.START,M.END+1):
        kp,sp,tp=kd/f"frame-{i:010d}.png",sd/f"frame-{i:08d}.png",td/f"frame-{i:010d}-trace.png"
        png(kp);png(sp);png(tp)
        packed,alpha=kd/f"frame-{i:010d}.packed-f32le.bin",kd/f"frame-{i:010d}.alpha-f32le.bin"
        packed.write_bytes(bytes(320_000));alpha.write_bytes(bytes(80_000)); ns=i*M.PTS_STEP*M.NANOS//30_000
        kf.append({"index":i,"timestamp_seconds":ns//M.NANOS,"timestamp_nanoseconds":ns%M.NANOS,"image":{**leaf(kp),"width":M.WIDTH,"height":M.HEIGHT},"production_map":{"packed":leaf(packed),"alpha":leaf(alpha)}})
        sf.append({"index":i,"pts":i*M.PTS_STEP,"time_base":"1/30000",**leaf(sp,"png")})
        tf.append({"index":i,"timestamp_seconds":ns//M.NANOS,"timestamp_nanoseconds":ns%M.NANOS,"base_png_sha256":sha(kp),"packed_sha256":sha(packed),"alpha_sha256":sha(alpha),"trace":leaf(tp)})
    kr={"schema":M.KJERAG_SCHEMA,"claim":M.KJERAG_CLAIM,"request":request,"view":M.OWNER_VIEW,"source":source,"build":build(PLAYBACK_SHA),"frames":kf,"run":run}
    krp=kd/"range-receipt.json";krp.write_text(json.dumps(kr))
    sr={"schema":M.STUDIO_SCHEMA,"authentication":M.CANONICAL_AUTH,"source":M.CANONICAL_STUDIO_SOURCE,"projection":{"input":"equirect","output":"flat","width":M.WIDTH,"height":M.HEIGHT,"yaw":M.OWNER_VIEW["yaw_degrees"],"pitch":M.OWNER_VIEW["pitch_degrees"],"horizontal_fov":M.OWNER_VIEW["fov_degrees"]},"interval":{"start_frame":M.START,"count":M.COUNT,"end_frame":M.END,"frames":sf},"extraction":{"temporal_interpolation":False}}
    srp=sd/"projection-receipt.json";srp.write_text(json.dumps(sr))
    ts=[{k:v for k,v in x.items() if k!="picked"} for x in source]
    tv={k:M.OWNER_VIEW[k] for k in ("yaw_radians","pitch_radians","fov_radians","yaw_degrees","pitch_degrees","fov_degrees","horizon_locked","readout","sampling","seam_band","exposure_tone")};tv.update({"seam":"factory","width":M.WIDTH,"height":M.HEIGHT})
    tr={"schema":M.TRACE_SCHEMA,"claim":M.TRACE_CLAIM,"input_receipt":{"path":str(krp),"bytes":krp.stat().st_size,"sha256":sha(krp),"schema":M.KJERAG_SCHEMA,"claim":M.KJERAG_CLAIM},"request":request,"view":tv,"source":ts,"build":build(TRACE_SHA),"input_run":run,"selected_frames":list(range(M.START,M.END+1)),"frames":tf}
    trp=td/"range-trace-receipt.json";trp.write_text(json.dumps(tr)); return krp,srp,trp
def pinned(path,label): return M.PinnedReceipt(path,sha(path),label)

class Tests(unittest.TestCase):
    def test_exact_owner_receipts_authenticate(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);kr,sr,tr=fixtures(root)
            for n in "kst": (root/n).mkdir()
            with pinned(kr,"Kjerag") as kp,pinned(sr,"Studio") as sp,pinned(tr,"trace") as tp:
                k=M.validate_kjerag(kp,root/"k",COMMIT,TREE,PLAYBACK_SHA);s=M.validate_studio(sp,root/"s",k);t=M.validate_trace(tp,root/"t",kp,k,COMMIT,TREE,TRACE_SHA)
            self.assertEqual((len(k["frames"]),len(s["frames"]),len(t["frames"])),(61,61,61))
    def test_arbitrary_source_view_build_refuse(self):
        cases=((lambda v:v["source"][0].update(sha256="f"*64),"canonical owner source"),(lambda v:v["view"].update(yaw_radians=0.0),"fixed owner view"),(lambda v:v["build"].update(runtime_git_tree="f"*40),"shipping build"),(lambda v:v["build"].update(runtime_tracked_tree_clean=False),"fully clean"))
        for mutate,msg in cases:
            with tempfile.TemporaryDirectory() as d:
                root=Path(d);kr,_,_=fixtures(root);v=json.loads(kr.read_text());mutate(v);kr.write_text(json.dumps(v));(root/"copy").mkdir()
                with pinned(kr,"K") as p,self.assertRaisesRegex(M.Refusal,msg): M.validate_kjerag(p,root/"copy",COMMIT,TREE,PLAYBACK_SHA)
    def test_fake_trace_pixels_refuse_private_derivation(self):
        f={"index":M.START,"sha256":"1"*64}; a={"input_receipt":{},"request":{},"view":{},"source":[],"input_run":{},"selected_frames":[],"build":build(TRACE_SHA),"frames":[dict(f) for _ in range(M.COUNT)]};b=json.loads(json.dumps(a));b["frames"][0]["sha256"]="2"*64
        with self.assertRaisesRegex(M.Refusal,"private derivation"): M.compare_trace_derivation(a,b)
    def test_pinned_tool_or_font_swap_refuses(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/"tool";p.write_bytes(b"trusted");q=M.PinnedFile(p,sha(p),"tool");r=p.with_name("r");r.write_bytes(b"malice!");os.replace(r,p)
            try:
                with self.assertRaisesRegex(M.Refusal,"path changed"):q.verify()
            finally:q.close()
    def test_parent_symlink_and_destination_race_refuse(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);real=root/"real";real.mkdir();link=root/"link";link.symlink_to(real)
            with self.assertRaises(OSError):os.open(link,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
            (real/"stage").mkdir();(real/"final").mkdir();fd=os.open(real,os.O_RDONLY|os.O_DIRECTORY)
            try:
                with self.assertRaisesRegex(M.Refusal,"already exists"):M.rename_noreplace(fd,"stage","final")
            finally:os.close(fd)
    def test_publication_uses_descriptor_pinned_parent(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);p=root/"parent";p.mkdir();fd=os.open(p,os.O_RDONLY|os.O_DIRECTORY);m=root/"moved";p.rename(m);p.mkdir();r=root/"rendered";r.mkdir()
            for n in (M.NATIVE,M.QUARTER,M.OUTPUT_RECEIPT):(r/n).write_bytes(n.encode())
            try:M.publish(fd,"result",r)
            finally:os.close(fd)
            self.assertTrue((m/"result"/M.OUTPUT_RECEIPT).exists());self.assertFalse((p/"result").exists())
    def test_refusal_closes_descriptors(self):
        before=len(os.listdir("/proc/self/fd"))
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);good=root/"good";good.write_text("{}");bad=root/"bad";bad.write_text("{")
            with self.assertRaises(M.Refusal),ExitStack() as stack:
                stack.enter_context(M.PinnedReceipt(good,sha(good),"first"));stack.enter_context(M.PinnedReceipt(bad,sha(bad),"second"))
            with self.assertRaises(M.Refusal),ExitStack() as stack:
                stack.enter_context(M.PinnedReceipt(good,sha(good),"first"));stack.enter_context(M.PinnedReceipt(good,sha(good),"second"));stack.enter_context(M.PinnedReceipt(bad,sha(bad),"third"))
            with self.assertRaises(M.Refusal):M.PinnedFile(bad,"0"*64,"bad hash")
        self.assertEqual(len(os.listdir("/proc/self/fd")),before)
    def test_x264_graph_and_sanitized_environment(self):
        self.assertEqual(M.x264_core(b"264 - core 164 r3108 31e19f9 - options","test"),"164 r3108 31e19f9")
        graph=M.filter_graph(Path("/private/font"));self.assertIn(M.FOOTER,graph);self.assertNotIn("rotate=",graph)
        env=M.clean_environment();self.assertEqual(env["PATH"],"/nonexistent")
        for key in ("LD_PRELOAD","LD_LIBRARY_PATH","FFREPORT","FONTCONFIG_FILE","FONTCONFIG_PATH"):self.assertNotIn(key,env)

if __name__=="__main__":unittest.main()
