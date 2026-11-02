"""Codebook recovery by black-box probing of an external AAC decoder (ffmpeg binary, oracle only).

Builds single-frame ADTS streams whose band data repeats a candidate bit string, decodes them with
`ffmpeg -f f32le`, least-squares fits the output to the IMDCT basis functions of the probed lines and
accepts the candidate as a codeword when the fit is exact, the copies agree and the implied sign/escape
bits are consistent. A breadth-first walk over bit strings then recovers each complete prefix code.

Usage: python3 derive.py <1..11 | 0 for scalefactors>   (writes cbN.json)
       python3 gen.py                                  (writes huffman_tables.rs)
Needs only the Python standard library and ffmpeg at /opt/homebrew/bin.
"""
import subprocess, struct, math, sys, os, tempfile

class BW:
    def __init__(self): self.bits=[]
    def w(self, v, n):
        for i in range(n-1,-1,-1): self.bits.append((v>>i)&1)
    def s(self, st):
        for c in st: self.bits.append(int(c))
    def bytes(self):
        b=self.bits+[0]*((-len(self.bits))%8)
        return bytes(int(''.join(map(str,b[i:i+8])),2) for i in range(0,len(b),8))

def adts(payload, sfi=3, ch=1):
    L=len(payload)+7
    w=BW(); w.w(0xFFF,12); w.w(0,1); w.w(0,2); w.w(1,1); w.w(1,2); w.w(sfi,4); w.w(0,1); w.w(ch,3)
    w.w(0,4); w.w(L,13); w.w(0x7FF,11); w.w(0,2)
    return w.bytes()+payload

def frame(cb, sfbits, specbits, nbands=3, G=100):
    w=BW()
    w.w(0,3); w.w(0,4); w.w(G,8)
    w.w(0,1); w.w(0,2); w.w(0,1); w.w(nbands,6); w.w(0,1)
    if nbands:
        w.w(cb,4); w.w(nbands,5)
        w.s(sfbits)
    w.w(0,1); w.w(0,1); w.w(0,1)
    w.s(specbits)
    w.w(7,3)
    return adts(w.bytes())

def silent():
    return frame(0,'','',nbands=0)

def run(frames):
    fd,path=tempfile.mkstemp(suffix='.aac'); os.write(fd,b''.join(frames)); os.close(fd)
    p=subprocess.run(['/opt/homebrew/bin/ffmpeg','-v','error','-f','aac','-i',path,'-f','f32le','-'],capture_output=True)
    os.unlink(path)
    n=len(p.stdout)//4
    return list(struct.unpack('<%df'%n,p.stdout)), p.stderr.decode()

N=2048
n0=(N/2+1)/2
def basis(k):
    return [math.sin(math.pi*(n+0.5)/N)*math.cos(2*math.pi/N*(n+n0)*(k+0.5)) for n in range(N)]
B=[basis(k) for k in range(12)]
def solve(A,b):
    n=len(A); M=[row[:]+[b[i]] for i,row in enumerate(A)]
    for c in range(n):
        p=max(range(c,n),key=lambda r:abs(M[r][c])); M[c],M[p]=M[p],M[c]
        for r in range(n):
            if r!=c:
                f=M[r][c]/M[c][c]
                for j in range(c,n+1): M[r][j]-=f*M[c][j]
    return [M[i][n]/M[i][i] for i in range(n)]
G_=[[sum(B[i][n]*B[j][n] for n in range(N)) for j in range(12)] for i in range(12)]
def fit(y):
    rhs=[sum(B[i][n]*y[n] for n in range(N)) for i in range(12)]
    x=solve(G_,rhs)
    res=sum((y[n]-sum(x[k]*B[k][n] for k in range(12)))**2 for n in range(N))
    en=sum(v*v for v in y)
    return x,res,en
if __name__=='__main__':
    spec=sys.argv[2]; cb=int(sys.argv[1])
    y,err=run([frame(cb,'000',spec),silent(),silent()])
    print(len(y),repr(err))
    x,res,en=fit(y[:2048])
    print(["%.4g"%v for v in x],res,en)
