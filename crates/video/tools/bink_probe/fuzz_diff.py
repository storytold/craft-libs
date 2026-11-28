"""Differential fuzzer: random synthetic Bink streams of every block type, picture size and
both revisions, decoded by ffmpeg and by the Rust decoder, and compared sample for sample.

    cargo build --release -p craft-video --features bink --example bink_binkdec
    python3 fuzz_diff.py "{}" 0 400          # options (a dict), first seed, number of cases

The options dict narrows what is generated, for example {'w': 17, 'h': 8, 'sig': b'BIKi'},
{'planes': [7, 8, 9]}, {'frames': 4, 'new16': 1, 'cover': 1}; see the opt.get calls. Use
'new16' with 'cover' so every block is drawn: where a 16x16 entry steps over blocks nothing
drew, ffmpeg shows whatever its recycled buffer held, which differs from run to run of content. Cases that
differ, or that a decoder doesn't finish, are kept in a new directory, made for the run in the one BINK_PROBE_CASES
names (default: the system's temporary directory) and printed at the end. BINK_PROBE_DECODER names the decoder program if it is not the
example in the workspace's target directory."""
import re, random, struct, subprocess, os, sys, tempfile
S=os.path.dirname(os.path.abspath(__file__))
W=os.path.normpath(os.path.join(S,'..','..','..','..'))
PARENT=os.environ.get('BINK_PROBE_CASES') or None
if PARENT: os.makedirs(PARENT,exist_ok=True)
CASES=tempfile.mkdtemp(prefix='bink_probe_',dir=PARENT)  # private to this user and this run
TIMEOUT=60  # seconds, for each decoder on one case
DECODER=os.environ.get('BINK_PROBE_DECODER') or os.path.join(W,'target','release','examples','bink_binkdec')
src=open(os.path.join(S,'..','..','src','bink','tables.rs')).read()
blk=src[src.index('TREES'):src.index('RUN_SCANS')]
pairs=[(int(a,16),int(b)) for a,b in re.findall(r'\(0x([0-9a-f]+), (\d+)\)',blk)]
TREES=[pairs[i*16:(i+1)*16] for i in range(16)]
assert len(pairs)==256
RUNS=[4,8,12,32]
class BW:
    def __init__(s): s.b=[]
    def put(s,v,n):
        for i in range(n): s.b.append((v>>i)&1)
    def align(s,n=32):
        while len(s.b)%n: s.b.append(0)
    def bytes(s):
        b=s.b+[0]*(-len(s.b)%8)
        return bytes(sum(b[i+j]<<j for j in range(8)) for i in range(0,len(b),8))
def build(sig,w,h,packets,flags=0,keys=None):
    n=len(packets); start=44+4*(n+1)
    packets=[p+b'\0'*(-len(p)%4) for p in packets]
    total=start+sum(map(len,packets))
    out=bytearray(sig)
    out+=struct.pack('<10I',total-8,n,max(map(len,packets)),n,w,h,25,1,flags,0)
    at=start
    for k,p in enumerate(packets):
        key=(k==0) if keys is None else keys[k]
        out+=struct.pack('<I',at|int(key)); at+=len(p)
    out+=struct.pack('<I',at)
    for p in packets: out+=p
    return bytes(out)
class Tree:
    def __init__(s,num,order): s.num=num; s.order=order
    def put(s,b,sym):
        code,ln=TREES[s.num][s.order.index(sym)]; b.put(code,ln)
def gen_tree(b,rnd,opt):
    mode=rnd.choice(opt.get('treemodes',['zero','list','merge','ident']))
    order=list(range(16))
    if mode=='zero':
        b.put(0,4); return Tree(0,order)
    num=rnd.randint(1,15); b.put(num,4)
    if mode=='ident':
        b.put(0,1); b.put(0,2)
        # depth 0: one pass merging runs of 1: need bits
        merged=[0]*16
        for st in range(0,16,2):
            b.put(0,1); 
        return Tree(num,order)
    if mode=='list':
        b.put(1,1); n=rnd.randint(0,7); b.put(n,3)
        first=rnd.sample(range(16),n+1)
        for x in first: b.put(x,4)
        return Tree(num,first+[x for x in range(16) if x not in first])
    b.put(0,1); depth=opt.get('depth',rnd.randint(0,3)); b.put(depth,2)
    for p in range(depth+1):
        size=1<<p; merged=[0]*16
        for start in range(0,16,size*2):
            a=start; bb=start+size
            for slot in range(start,start+size*2):
                if a==start+size: fb=1
                elif bb==start+size*2: fb=0
                else:
                    fb=rnd.randint(0,1); b.put(fb,1)
                if fb: merged[slot]=order[bb]; bb+=1
                else: merged[slot]=order[a]; a+=1
        order=merged
    return Tree(num,order)
def L(n): return (n+511).bit_length()
def list_gen(b,rnd,coef,planes,budget,p1):
    """Mirror of the list walk; random bits generated and written."""
    def bit():
        v=1 if rnd.random()<p1 else 0; b.put(v,1); return v
    lst=[('P',0,2),('P',2,0),('P',2,2)]
    lst+= [('S',0,1),('S',1,0),('S',1,1)] if coef else [('G',0,0)]
    order=[]
    bud=[budget]
    for plane in range(planes-1,-1,-1):
        if not coef:
            for _ in order:
                if bit():
                    bud[0]-=1
                    if bud[0]==0: return
        def value():
            if coef:
                for _ in range(plane): bit()
            bit()
            if not coef:
                order.append(1); bud[0]-=1
                return bud[0]==0
            return False
        i=0
        while i<len(lst):
            k,r,c=lst[i]
            if not bit(): i+=1; continue
            if k in 'PG':
                if k=='P': lst[i]=('R',0 if r==0 else 4,0 if c==0 else 4)
                else: del lst[i]
                for dr,dc in ((0,0),(0,1),(1,0),(1,1)):
                    if bit(): lst.insert(0,('S',r+dr,c+dc)); i+=1
                    elif value(): return
            elif k=='R':
                lst[i]=('G',r,c); lst+=[('G',r,c+2),('G',r+2,c),('G',r+2,c+2)]
            else:
                del lst[i]
                if value(): return
def gen_plane(b,rnd,bw,bh,inter,opt,subn=None):
    if subn is None: subn=bw//2
    names=['bt','sub']+['ch%d'%i for i in range(16)]+['cl','pat','x','y','run']
    T={n:gen_tree(b,rnd,opt) for n in names}
    intra_types=opt.get('intra',[3,5,6,8,9])
    inter_types=opt.get('inter',[0,2,4,7])
    # plan blocks
    rows=[]  # per row: list of entries (type, dict)
    need={k:[0]*bh for k in ('bt','sub','col','pat','x','y','idc','pdc','run')}
    vals={k:[[] for _ in range(bh)] for k in need}
    inl=[BW() for _ in range(bh)]
    above16=set()
    def intra(t,by):
        ib=inl[by]
        if t==3:
            ib.put(rnd.randint(0,15),4); i=0
            fixed=opt.get('runfixed')
            assert fixed is None or fixed>0,'runfixed must be positive'
            while i<63:
                run=min(rnd.choice([1,1,2,3,5,8,12,16,16]),64-i) if fixed is None else min(fixed,64-i)
                run=min(run,16)
                if opt.get('run63') and i==0: run=63
                vals['run'][by].append(run-1)
                if rnd.random()<0.5: ib.put(1,1); vals['col'][by].append(rnd.randint(0,255))
                else:
                    ib.put(0,1); vals['col'][by]+= [rnd.randint(0,255) for _ in range(run)]
                i+=run
            if i==63: vals['col'][by].append(rnd.randint(0,255))
        elif t==5:
            vals['idc'][by].append(None)
            planes=rnd.choice(opt.get('planes',[0,1,2,3,4,5,6])); ib.put(planes,4)
            list_gen(ib,rnd,True,planes,0,rnd.choice([0.1,0.3,0.5,0.7]))
            ib.put(rnd.randint(0,15),4)
        elif t==6: vals['col'][by].append(rnd.randint(0,255))
        elif t==8:
            vals['col'][by]+=[rnd.randint(0,255),rnd.randint(0,255)]
            vals['pat'][by]+=[rnd.randint(0,255) for _ in range(8)]
        elif t==9: vals['col'][by]+=[rnd.randint(0,255) for _ in range(64)]
    for by in range(bh):
        bx=0
        while bx<bw:
            cands=list(intra_types)+(inter_types if inter else [])
            ok16=(by%2==0 and (bx+1<bw or opt.get('edgex')) and (by+1<bh or opt.get('edgey')) and (bx%2==0 or opt.get('oddcol')))
            if opt.get('new16'):
                cov=True
                if opt.get('cover'):
                    if by%2==0: cov=(bx%2==0)
                    elif bx%2==0: cov=(bx,by-1) in above16
                    else: cov=(bx-1,by-1) in above16 and (bx+1,by-1) in above16
                if cov and rnd.random()<opt.get('p16',0.2): t=1
                else: t=rnd.choice(cands)
            elif opt.get('odd16')=='mirror':
                if by%2==1 and (bx,by-1) in above16: t=1
                else:
                    if ok16 and rnd.random()<opt.get('p16',0.2): t=1
                    else: t=rnd.choice(cands)
                    if by%2==1 and t!=1:
                        # must not overlap? allow anything
                        pass
            else:
                if (ok16 or by%2==1 or (opt.get('ffodd') and bx%2==1)) and rnd.random()<opt.get('p16',0.2): t=1
                else: t=rnd.choice(cands)
            vals['bt'][by].append(t)
            if t==1:
                if by%2==0 and not ((opt.get('ffodd') or opt.get('new16')) and bx%2==1):
                    above16.add((bx,by))
                    st=rnd.choice(opt.get('sub',[3,5,6,8,9])); vals['sub'][by].append(st); intra(st,by)
                bx+=2; continue
            if t in (2,4,7):
                # mv inside padded plane
                m=opt.get('mvmax',15)
                dx=rnd.randint(max(-m,-bx*8),min(m,(bw-1-bx)*8)); dy=rnd.randint(max(-m,-by*8),min(m,(bh-1-by)*8))
                if opt.get('mvzero'): dx=dy=0
                vals['x'][by].append(dx); vals['y'][by].append(dy)
                if t==7:
                    vals['pdc'][by].append(None)
                    planes=rnd.choice(opt.get('planes',[0,1,2,3,4,5,6])); inl[by].put(planes,4)
                    list_gen(inl[by],rnd,True,planes,0,rnd.choice([0.1,0.3,0.5,0.7]))
                    inl[by].put(rnd.randint(0,15),4)
                elif t==4:
                    budget=rnd.randint(0,127); tp=rnd.randint(0,7)
                    inl[by].put(budget,7); inl[by].put(tp,3)
                    list_gen(inl[by],rnd,False,tp+1,budget+1,rnd.choice([0.1,0.3,0.5,0.7,0.9]))
            elif t!=0: intra(t,by)
            bx+=1
    if opt.get('dbg'): print('plane',bw,bh,{k:v for k,v in vals.items() if any(v)})
    widths={'bt':L(bw),'sub':L(subn),'col':L(bw*64),'pat':L(bw*8),'x':L(bw),'y':L(bw),'idc':L(bw),'pdc':L(bw),'run':L(bw*48)}
    if opt.get('wadj') and opt['wadj'][0]==bw:
        for k,dlt in zip(('bt','sub','col','pat','x','y','idc','pdc','run'),opt['wadj'][1:]): widths[k]+=dlt
    left={k:0 for k in need}; done={k:False for k in need}
    cstate=[0]
    def emit_chunk(k,v):
        n=len(v)
        memset=opt.get('memset',0.5)
        if k in ('bt','sub'):
            if len(set(v))==1 and rnd.random()<memset: b.put(1,1); b.put(v[0],4); return
            b.put(0,1); last=0; i=0
            while i<n:
                run=0
                while i+run<n and v[i+run]==last: run+=1
                c=[j for j,r in enumerate(RUNS) if r<=run]
                if c and rnd.random()<0.8:
                    j=rnd.choice(c); T[k].put(b,12+j); i+=RUNS[j]
                else:
                    T[k].put(b,v[i]); last=v[i]; i+=1
        elif k=='col':
            def one(c):
                T['ch%d'%cstate[0]].put(b,c>>4); T['cl'].put(b,c&15); cstate[0]=c>>4
            if len(set(v))==1 and rnd.random()<memset: b.put(1,1); one(v[0]); return
            b.put(0,1)
            for c in v: one(c)
        elif k=='pat':
            for p in v: T['pat'].put(b,p&15); T['pat'].put(b,p>>4)
        elif k in ('x','y'):
            if len(set(v))==1 and rnd.random()<memset:
                b.put(1,1); b.put(abs(v[0]),4)
                if v[0]: b.put(int(v[0]<0),1)
                return
            b.put(0,1)
            for d in v:
                T[k].put(b,abs(d))
                if d: b.put(int(d<0),1)
        elif k in ('idc','pdc'):
            if k=='idc': val=rnd.randint(0,2047); b.put(val,11)
            else:
                val=rnd.randint(-1023,1023); b.put(abs(val),10)
                if val: b.put(int(val<0),1)
            cnt=1
            while cnt<n:
                g=min(8,n-cnt); nb=rnd.choice(opt.get('dcbits',[0,0,1,2,3,5,8])); b.put(nb,4)
                for _ in range(g):
                    if nb:
                        d=rnd.randint(0,(1<<nb)-1); b.put(d,nb)
                        if d:
                            sg=rnd.randint(0,1)
                            if k=='idc' and not opt.get('dcwild'):
                                if val-d<0: sg=0
                                elif val+d>2047: sg=1
                            b.put(sg,1); val+=-d if sg else d
                cnt+=g
        elif k=='run':
            if len(set(v))==1 and rnd.random()<memset: b.put(1,1); b.put(v[0],4); return
            b.put(0,1)
            for r in v: T['run'].put(b,r)
    for by in range(bh):
        for k in ('bt','sub','col','pat','x','y','idc','pdc','run'):
            if done[k] or left[k]>0:
                left[k]-=len(vals[k][by]); assert left[k]>=0; continue
            # new chunk
            tot=sum(len(vals[k][j]) for j in range(by,bh))
            if tot==0:
                b.put(0,widths[k]); done[k]=True; continue
            mx=(1<<widths[k])-1
            if opt.get('maxchunk'): mx=min(mx,opt['maxchunk'](k,bw))
            v=[]; j=by
            while j<bh:
                nv=vals[k][j]
                if v and (len(v)+len(nv)>mx or rnd.random()<opt.get('pcut',0.5)): break
                v=v+nv; j+=1
            assert 0<len(v)<=mx,(k,len(v),mx)
            b.put(len(v),widths[k]); emit_chunk(k,v)
            left[k]=len(v)-len(vals[k][by])
        b.b+=inl[by].b
def gen_file(rnd,opt):
    w=opt.get('w') or rnd.choice([rnd.randint(1,40),rnd.randint(1,100),rnd.choice([8,16,24,32,48,64,19,33,68,72,80])])
    h=opt.get('h') or rnd.choice([rnd.randint(1,40),rnd.randint(1,70),rnd.choice([8,16,24,32,48,9,17,25])])
    sig=opt.get('sig') or rnd.choice([b'BIKi',b'BIKf'])
    nf=opt.get('frames',3)
    pk=[]
    for f in range(nf):
        b=BW()
        if sig==b'BIKi': b.put(0,32)
        cw,ch=(w+1)//2,(h+1)//2
        for (pw,ph,sn) in ((w,h,(w+7)//16),(cw,ch,(w//2+7)//16),(cw,ch,(w//2+7)//16)):
            gen_plane(b,rnd,(pw+7)//8,(ph+7)//8,f>0 or opt.get('inter0'),opt,sn)
            b.align(32)
        pk.append(b.bytes())
    return sig,w,h,pk
def run_case(seed,opt,keep=False):
    rnd=random.Random(seed)
    sig,w,h,pk=gen_file(rnd,opt)
    path=os.path.join(CASES,'c%d_%d.bik'%(os.getpid(),seed))
    open(path,'wb').write(build(sig,w,h,pk))
    try: return compare(seed,opt,keep,sig,w,h,pk,path)
    except subprocess.TimeoutExpired as e:
        # the input stays for a look
        return dict(seed=seed,sig=sig,w=w,h=h,res='timeout',cmd=e.cmd[0],path=path)
    finally:
        for out in (path+'.yuv',path+'.old'):
            if os.path.exists(out): os.remove(out)
def compare(seed,opt,keep,sig,w,h,pk,path):
    r=subprocess.run(['ffmpeg','-v','error','-i',path,'-f','rawvideo','-pix_fmt','yuv420p','-fps_mode','passthrough','-'],capture_output=True,timeout=TIMEOUT)
    ff=r.stdout; fferr=r.stderr.decode(errors='replace')
    r2=subprocess.run([opt.get('bin',DECODER),path,path+'.yuv'],capture_output=True,env=dict(os.environ,**opt.get('env',{})),timeout=TIMEOUT)
    rs=open(path+'.yuv','rb').read() if os.path.exists(path+'.yuv') else b''
    rlog=r2.stdout.decode()+r2.stderr.decode()
    fs=w*h+2*((w+1)//2)*((h+1)//2)
    ffclean=(fferr=='' and len(ff)==fs*len(pk) and r.returncode==0)
    rsclean=('ERR' not in rlog and len(rs)==fs*len(pk) and r2.returncode==0)
    if ffclean and rsclean: res='same' if ff==rs else 'DIFF'
    elif ffclean: res='RUSTERR'
    elif rsclean: res='fferr'
    else: res='botherr'
    info=dict(seed=seed,sig=sig,w=w,h=h,res=res,fferr=fferr[:300],rlog=rlog[:300],path=path)
    if res=='DIFF':
        for f in range(len(pk)):
            a=ff[f*fs:(f+1)*fs]; c=rs[f*fs:(f+1)*fs]
            if a!=c:
                d=[i for i in range(fs) if a[i]!=c[i]]
                info['frame']=f; info['ndiff']=len(d); info['first']=d[:5]; info['lumasize']=w*h
                break
    if opt.get('old'):
        r3=subprocess.run([opt['old'],path,path+'.old'],capture_output=True,timeout=TIMEOUT)
        # it may reject a case before it makes its output
        ro=open(path+'.old','rb').read() if os.path.exists(path+'.old') else b''
        info['oldok']=('ERR' not in r3.stdout.decode() and len(ro)==fs*len(pk) and r3.returncode==0)
        info['oldsame']=(ro==ff) if ffclean else None
        if ffclean and info['oldok'] and info['oldsame'] and res!='same': info['res']=res='REGRESS'
    if not keep and res in ('same','botherr'):
        os.remove(path)
    return info
if __name__=='__main__':
    import json
    from concurrent.futures import ThreadPoolExecutor
    opt=eval(sys.argv[1]); a=int(sys.argv[2]); n=int(sys.argv[3])
    with ThreadPoolExecutor(8) as ex: res=list(ex.map(lambda s: run_case(s,opt),range(a,a+n)))
    from collections import Counter
    print(Counter(r['res'] for r in res))
    for r in res:
        if r['res'] not in ('same',): print({k:v for k,v in r.items() if k!='path'})
    if os.listdir(CASES): print('kept cases:',CASES)
    else: os.rmdir(CASES)
