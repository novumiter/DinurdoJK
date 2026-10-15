"""Build a native oracle from a TaystJK checkout and generate Rust test fixtures.

Usage: python scripts/verify-patch-lod-reference.py --reference-dir PATH
Requires Visual Studio C++ tools on Windows. Output is under target/patch-lod.
The oracle uses the actual tr_curve/tr_bsp/tr_surface bodies, not a second port.
The upstream reversed-stitch out-of-bounds error reads are sanitized to k-1.
"""
import argparse
import hashlib
import pathlib
import subprocess

HEADER = r'''
#include <cmath>
#include <cstring>
#include <cstdlib>
#include <cstdio>
#include <vector>
#define MAX_GRID_SIZE 65
#define MAX_PATCH_SIZE 32
#define MAXLIGHTMAPS 4
#define TAG_GRIDMESH 0
#define qfalse 0
#define qtrue 1
#define SF_GRID 1
typedef int qboolean;
typedef float vec3_t[3];
struct drawVert_t { float xyz[3],st[2],lightmap[4][2],normal[3]; unsigned char color[4][4]; };
typedef int surfaceType_t;
struct srfGridMesh_s { int surfaceType,width,height,lodFixed,lodStitched; float *widthLodError,*heightLodError; float meshBounds[2][3],localOrigin[3],meshRadius,lodOrigin[3],lodRadius; drawVert_t verts[1]; };
typedef srfGridMesh_s srfGridMesh_t;
struct msurface_t { void *data; };
struct world_t { int numsurfaces; msurface_t *surfaces; };
struct Cvar { float value; };
Cvar subdivisions,curveError;
Cvar *r_subdivisions=&subdivisions,*r_lodCurveError=&curveError;
struct Ori { float axis[3][3],origin[3]; };
struct Backend { Ori ori; struct { Ori ori; } viewParms; } backEnd;
#define DotProduct(a,b) ((a)[0]*(b)[0]+(a)[1]*(b)[1]+(a)[2]*(b)[2])
#define VectorLengthSquared(a) DotProduct(a,a)
#define VectorLength(a) std::sqrt(DotProduct(a,a))
#define VectorCopy(a,b) do { for(int z=0;z<3;z++)(b)[z]=(a)[z]; } while(0)
#define VectorClear(a) do { for(int z=0;z<3;z++)(a)[z]=0; } while(0)
#define VectorSubtract(a,b,c) do { for(int z=0;z<3;z++)(c)[z]=(a)[z]-(b)[z]; } while(0)
#define VectorAdd(a,b,c) do { for(int z=0;z<3;z++)(c)[z]=(a)[z]+(b)[z]; } while(0)
#define VectorScale(a,b,c) do { for(int z=0;z<3;z++)(c)[z]=(a)[z]*(b); } while(0)
float VectorNormalize2(const float *a,float *b) { float length=VectorLength(a); float inv=length?1.f/length:0; for(int z=0;z<3;z++)b[z]=a[z]*inv; return length; }
float VectorNormalize(float *a) { return VectorNormalize2(a,a); }
void CrossProduct(float *a,float *b,float *c) { c[0]=a[1]*b[2]-a[2]*b[1];c[1]=a[2]*b[0]-a[0]*b[2];c[2]=a[0]*b[1]-a[1]*b[0]; }
void ClearBounds(float *a,float *b) { for(int z=0;z<3;z++){a[z]=1e30f;b[z]=-1e30f;} }
void AddPointToBounds(float *v,float *a,float *b) { for(int z=0;z<3;z++){ if(v[z]<a[z])a[z]=v[z]; if(v[z]>b[z])b[z]=v[z]; } }
void *Z_Malloc(int size,int,int) { return malloc(size); }
void Z_Free(void *p) { free(p); }
'''

TAIL = r'''
void dump(int kind,float sub,srfGridMesh_t *g) {
 printf("G %d %.9g %d %d\n",kind,sub,g->width,g->height);
 for(int x=0;x<g->width;x++)printf("%.9g ",g->widthLodError[x]);puts("");
 for(int y=0;y<g->height;y++)printf("%.9g ",g->heightLodError[y]);puts("");
 for(int i=0;i<g->width*g->height;i++){
  auto &v=g->verts[i];
  for(float f:v.xyz)printf("%.9g ",f);for(float f:v.st)printf("%.9g ",f);
  for(auto &uv:v.lightmap)for(float f:uv)printf("%.9g ",f);
  for(float f:v.normal)printf("%.9g ",f);for(auto &c:v.color)for(auto f:c)printf("%u ",f);puts("");
 }
 // Native LodErrorForVolume: negative, zero, near, far, behind and lateral views.
 for(float err:{-1.f,0.f,25.f,250.f,2500.f})for(int view=0;view<4;view++){
  curveError.value=err;for(int i=0;i<3;i++){backEnd.ori.axis[i][i]=1;g->lodOrigin[i]=float(i*16);backEnd.viewParms.ori.origin[i]=0;}
  backEnd.viewParms.ori.origin[0]= view==0?1.f:view==1?4096.f:view==2?-4096.f:0.f;
  VectorClear(backEnd.viewParms.ori.axis[0]);backEnd.viewParms.ori.axis[0][view==3?1:0]=1.f;
  float t=LodErrorForVolume(g->lodOrigin,32.f);printf("L %.9g %d %.9g",err,view,t);
  int w=2,h=2;for(int i=1;i<g->width-1;i++)if(g->widthLodError[i]<=t)w++;for(int i=1;i<g->height-1;i++)if(g->heightLodError[i]<=t)h++;
  printf(" %d %d\n",w,h);
 }
 puts("END");
}
int main(){
 for(int kind=0;kind<4;kind++)for(float sub:{0.f,1.f,4.f,12.f,80.f}){
  int w=kind==0?5:kind==1?3:kind==2?5:9,h=kind==2?7:3;
  drawVert_t points[1024]={};int xs[9]={1,1,0,-1,-1,-1,0,1,1},ys[9]={0,1,1,1,0,-1,-1,-1,0};
  for(int y=0;y<h;y++)for(int x=0;x<w;x++){
   auto &v=points[y*w+x];v.xyz[0]=float(x*32);v.xyz[1]=float(y*32);
   v.xyz[2]=kind==1&&x==1&&y==1?512.f:kind==2?float(x*x*11+y*y*19+(x%2)*85-(y%2)*33):0.f;
   if(kind==3){v.xyz[0]=float(xs[x]*128);v.xyz[1]=float(ys[x]*128);v.xyz[2]=float(y*64);}
   v.st[0]=x*.25f;v.st[1]=y*.125f;v.normal[2]=1;
   for(int l=0;l<4;l++){v.lightmap[l][0]=x*.0625f+l*.125f;v.lightmap[l][1]=y*.0625f;for(int c=0;c<4;c++)v.color[l][c]=(x*17+y*11+l*13+c*7)%256;}
  }
  subdivisions.value=sub;auto g=R_SubdividePatchToGrid(w,h,points);dump(kind,sub,g);R_FreeSurfaceGridMesh(g);
 }
 for(int orientation=0;orientation<4;orientation++)for(int enabled=0;enabled<2;enabled++){
  drawVert_t ctrl[65][65]={};float errors[2][65]={};srfGridMesh_t *grids[2];
  for(int grid=0;grid<2;grid++){
   int w=grid==0?((orientation&2)?3:5):3,h=grid==0&&(orientation&2)?5:3;
   for(int y=0;y<h;y++)for(int x=0;x<w;x++){
    float px=float(x*(grid==0?1:2));float py=float(y+(grid==0?0:-2));
    if(grid==0&&(orientation&2)){px=float(y);py=float(x);}
    if(grid==0&&(orientation&1))px=4-px;
    auto &v=ctrl[y][x];v.xyz[0]=px;v.xyz[1]=py;v.xyz[2]=0;
    v.st[0]=x*.25f;v.st[1]=y*.125f;v.normal[2]=1;
    for(int l=0;l<4;l++){v.lightmap[l][0]=x*.0625f;v.lightmap[l][1]=y*.0625f;for(int c=0;c<4;c++)v.color[l][c]=(x*17+y*11+l*13+c*7)%256;}
   }
   for(int i=0;i<w;i++)errors[0][i]=i==0||i==w-1?0:float(i*(grid==0?((orientation&2)?.5:1):.2));
   for(int i=0;i<h;i++)errors[1][i]=i==0||i==h-1?0:float(i*(grid==0&&(orientation&2)?1:.5));
   grids[grid]=R_CreateSurfaceGridMesh(w,h,ctrl,errors);grids[grid]->lodRadius=10;VectorClear(grids[grid]->lodOrigin);
  }
  msurface_t surfaces[2]={{grids[0]},{grids[1]}};world_t world={2,surfaces};
  if(enabled)R_StitchAllPatches(world);R_FixSharedVertexLodError(world);
  for(int i=0;i<2;i++){auto g=(srfGridMesh_t*)world.surfaces[i].data;dump(10+orientation*2+i,float(enabled),g);R_FreeSurfaceGridMesh(g);}
 }
}
'''

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference-dir', required=True, type=pathlib.Path)
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parent.parent
    out = root / 'target/patch-lod'
    out.mkdir(parents=True, exist_ok=True)
    ref = args.reference_dir / 'codemp/rd-vanilla'
    curve = (ref/'tr_curve.cpp').read_text()
    bsp = (ref/'tr_bsp.cpp').read_text()
    surface = (ref/'tr_surface.cpp').read_text()
    body = curve[curve.index('static void LerpDrawVert'):]
    stitch = bsp[bsp.index('int R_MergedWidthPoints'):bsp.index('void R_MovePatchSurfacesToHunk')]
    # This is the one documented memory-safety correction in the port.
    stitch = stitch.replace('grid1->verts[k - 1 + offset1].xyz, grid1->widthLodError[k+1]', 'grid1->verts[k - 1 + offset1].xyz, grid1->widthLodError[k-1]')
    stitch = stitch.replace('grid1->verts[grid1->width * (k - 1) + offset1].xyz, grid1->heightLodError[k+1]', 'grid1->verts[grid1->width * (k - 1) + offset1].xyz, grid1->heightLodError[k-1]')
    # The slice ends inside the next function's comment: trim that comment.
    stitch = stitch[:stitch.rfind('/*')]
    volume = surface[surface.index('static float\tLodErrorForVolume'):surface.index('void RB_SurfaceGrid')]
    volume = volume[:volume.rfind('/*')]
    (out/'oracle.cpp').write_text(curve[:curve.index('#include')]+HEADER+body+stitch+volume+TAIL)
    vswhere = pathlib.Path(r'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe')
    install = subprocess.check_output([str(vswhere),'-latest','-property','installationPath'],text=True).strip()
    bat = out/'build-oracle.cmd'
    bat.write_text(f'@echo off\ncall "{install}\\VC\\Auxiliary\\Build\\vcvars64.bat" >nul\ncl /nologo /EHsc /O2 /fp:strict oracle.cpp /Fe:oracle.exe\n')
    subprocess.run(['cmd','/c',str(bat)],cwd=out,check=True)
    golden = subprocess.check_output([str(out/'oracle.exe')])
    (out/'reference-grids.txt').write_bytes(golden)
    hashes = {f:hashlib.sha256((ref/f).read_bytes()).hexdigest() for f in ['tr_curve.cpp','tr_bsp.cpp','tr_surface.cpp','tr_init.cpp']}
    (out/'reference-hashes.txt').write_text('\n'.join(f'{h}  {f}' for f,h in hashes.items())+'\n')
    print(f'Generated {len(golden)} bytes of native-reference results in {out}')

if __name__ == '__main__':
    main()
