# RT AA cleanup — 2026-10-09

Switching from 2x MSAA to global TAA selects 1x attachments. The ordinary model
pipelines were invalidated, but `ensure_rt_model_receivers` kept the old RT model
receiver pipelines installed while its replacement worker ran. Their shared
label is `JKA dynamic model opaque pipeline`, so the validation error named that
pipeline even though the stale cache was the RT receiver cache.

RT model receivers are now discarded immediately when their attachment format,
sample count, fog specialization or map-light specialization changes. Models use
their ordinary lighting path while the asynchronous replacement compiles. A
matching receiver stays installed. This preserves the normal MSAA/TAA switch and
does not change the RT lighting/shadow equations.

The radiance denoiser was retired after the user reported that both modes cost
substantially more FPS than global TAA or regional RTAA. Its implementation added
a separate full-resolution stochastic-light ray pass, temporal history processing
and, for SVGF, three spatial passes. Receivers unable to use its dominant-light
estimate still used direct queries. Inspection found no small accidental-work
bug that would remove those principal costs.

Removed the denoiser compute shaders/resources, receiver bindings 17–23, filtered
world-shader variants, runtime/config cvar, renderer command, menu controls and
profiler intervals. Old `r_rtDenoiser` config entries are ignored and are not
archived again. The scalar reduced-rate sun-shadow cache, tinted-glass RGB
transmission, RT sample-count control, global TAA and `r_rtTaa` remain.

Validation:

- Vulkan regression reproduced the reported 2x-pipeline/1x-attachment error, then
  passed six cache/fallback transitions covering MSAA, LDR/HDR format, fog and
  map-light specialization changes.
- Actual hardware RT model pipeline and reduced sun-cache GPU checks passed.
- RT and raster tinted-glass acceptance GPU checks passed.
- Regional temporal filtering GPU check passed, including unchanged pixels
  outside the mask and invalid-history rejection.
- Renderer shader/unit checks passed; the previously known unrelated minimal
  world shader performance audit remains excluded.
- Configuration tests confirm the removed denoiser is unregistered/unarchived
  and the regional option still preserves MSAA.

The user reported regional RTAA outperforming global TAA away from dynamic
lights, with similar FPS when a dynamic light covers the view. These are user
observations, not a new controlled benchmark of this cleanup build.
