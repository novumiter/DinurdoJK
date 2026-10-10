// Reproduces Saltreach src/solver-kernels.ts `pressure` in JavaScript and emits
// the 4x3 one-step fixture used by jka-water::coastal. Keep f32 writes at the same
// points as the upstream Float32Array implementation.
const nx=4,nz=3,dx=.3,dz=.4,dt=.025,n=nx*nz;
const bed=Float32Array.from([-1,-.8,-.2,.15,-1.1,-.7,-.1,.25,-1.2,-.9,-.3,.35]);
const h=Float32Array.from([1.05,.83,.24,0,1.08,.76,.14,0,1.18,.91,.27,0]);
const u=Float32Array.from([.11,-.07,.03,0,-.04,.09,-.02,0,.06,-.05,.01,0]);
const v=Float32Array.from([-.03,.02,.04,0,.05,-.06,.01,0,0,0,0,0]);
const fluxX=new Float32Array(n),fluxZ=new Float32Array(n),limit=new Float32Array(n),next=new Float32Array(n);
const cap=(x,a,b)=>Math.max(a,Math.min(b,x));
const smooth=(a,b,x)=>{const t=cap((x-a)/(b-a),0,1);return t*t*(3-2*t)};
for(let j=0;j<nz;j++)for(let i=0;i<nx;i++){
 const k=j*nx+i,eta=bed[k]+h[k];
 if(i<nx-1){const q=k+1,e2=bed[q]+h[q],crest=Math.max(bed[k],bed[q]),faceH=Math.max(0,Math.max(eta,e2)-crest);if(faceH<.001){u[k]=0;fluxX[k]=0}else{const drag=.065+.08/(faceH+.075);u[k]=cap((u[k]-9.81*dt*(e2-eta)/dx)/(1+dt*drag),-5,5);const donor=u[k]>0?Math.max(0,eta-crest):Math.max(0,e2-crest);fluxX[k]=u[k]*donor;if(h[k]>.008&&h[q]>.008)fluxX[k]-=.32*smooth(.12,.48,Math.abs(e2-eta)/dx)*(e2-eta)/dx}}else fluxX[k]=0;
 if(j<nz-1){const q=k+nx,e2=bed[q]+h[q],crest=Math.max(bed[k],bed[q]),faceH=Math.max(0,Math.max(eta,e2)-crest);if(faceH<.001){v[k]=0;fluxZ[k]=0}else{const drag=.065+.08/(faceH+.075);v[k]=cap((v[k]-9.81*dt*(e2-eta)/dz)/(1+dt*drag),-5,5);const donor=v[k]>0?Math.max(0,eta-crest):Math.max(0,e2-crest);fluxZ[k]=v[k]*donor;if(h[k]>.008&&h[q]>.008)fluxZ[k]-=.32*smooth(.12,.48,Math.abs(e2-eta)/dz)*(e2-eta)/dz}}else fluxZ[k]=0;
}
for(let j=0;j<nz;j++)for(let i=0;i<nx;i++){const k=j*nx+i,outgoing=(Math.max(fluxX[k],0)+(i?Math.max(-fluxX[k-1],0):0))/dx+(Math.max(fluxZ[k],0)+(j?Math.max(-fluxZ[k-nx],0):0))/dz;limit[k]=outgoing>0?Math.min(1,h[k]/(dt*outgoing+1e-10)):1}
for(let k=0;k<n;k++){fluxX[k]*=fluxX[k]>=0?limit[k]:limit[Math.min(k+1,n-1)];fluxZ[k]*=fluxZ[k]>=0?limit[k]:limit[Math.min(k+nx,n-1)]}
for(let j=0;j<nz;j++)for(let i=0;i<nx;i++){const k=j*nx+i;next[k]=Math.max(0,h[k]-dt*((fluxX[k]-(i?fluxX[k-1]:0))/dx+(fluxZ[k]-(j?fluxZ[k-nx]:0))/dz))}
for(const [name,array] of Object.entries({u,v,fluxX,fluxZ,limit,next}))console.log(`${name}: [${Array.from(array,x=>`${x.toPrecision(9)}f32`).join(', ')}]`);
