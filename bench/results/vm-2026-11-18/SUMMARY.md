| metric | composefs-host | composefs-sha512-host | plain-host | composefs-Skylake-Client-v4 | composefs-sha512-Skylake-Client-v4 | plain-Skylake-Client-v4 |
|---|---|---|---|---|---|---|
| boot kernel (s) | 0.362 | 0.361 | 0.359 | 0.364 | 0.377 | 0.365 |
| boot initrd (s) | 2.041 | 2.092 | 1.986 | 2.177 | 2.157 | 2.058 |
| boot userspace (s) | 2.712 | 3.217 | 1.992 | 2.813 | 3.169 | 2.066 |
| boot total (s) | 5.117 | 5.672 | 4.338 | 5.355 | 5.704 | 4.489 |
| boot graphical (s) | 2.712 | 3.217 | 1.990 | 2.812 | 3.129 | 2.065 |
| read_closure_cold (s, median of 3) | 12.667 | 16.135 | 8.878 | 18.037 | 16.931 | 9.394 |
| read_closure_warm (s, median of 3) | 7.904 | 10.483 | 5.534 | 12.289 | 11.279 | 6.376 |
| walk_closure_cold (s, median of 3) | 2.479 | 2.588 | 3.795 | 3.715 | 3.737 | 4.847 |
| walk_closure_warm (s, median of 3) | 1.333 | 1.326 | 1.389 | 2.458 | 2.455 | 2.524 |
| app_perl_cold (s, median of 3) | 0.053 | 0.063 | 0.042 | 0.064 | 0.062 | 0.043 |
| app_perl_warm (s, median of 3) | 0.013 | 0.012 | 0.012 | 0.014 | 0.014 | 0.014 |
| app_konsole_cold (s, median of 3) | 0.328 | 0.350 | 0.315 | 0.399 | 0.378 | 0.308 |
| app_konsole_warm (s, median of 3) | 0.070 | 0.069 | 0.070 | 0.077 | 0.077 | 0.076 |
| app_kate_cold (s, median of 3) | 0.299 | 0.369 | 0.397 | 0.353 | 0.364 | 0.368 |
| app_kate_warm (s, median of 3) | 0.064 | 0.063 | 0.062 | 0.073 | 0.073 | 0.071 |
| app_firefox_cold (s, median of 3) | 0.871 | 1.030 | 0.767 | 1.109 | 1.003 | 0.847 |
| app_firefox_warm (s, median of 3) | 0.444 | 0.450 | 0.448 | 0.509 | 0.493 | 0.512 |
| verify_check_contents (s, median of 3) | 39.011 | 47.960 | 30.168 | 59.739 | 58.666 | 45.599 |
| fs_used | 10658607104 | 10945650688 | 9985576960 | 10666835968 | 10946613248 | 9992949760 |
| store_MiB | 0 | 0 | 8657 | 0 | 0 | 8657 |
| objects_MiB | 8610 | 8629 |  | 8610 | 8629 |  |
| images_MiB | 1 | 1 |  | 1 | 1 |  |
| image_bytes | 85557248 | 105365504 |  | 85557248 | 105365504 |  |
| objects | 206537 | 206537 |  | 206537 | 206537 |  |
| fetch_s | 20.200 | 20.610 | 19.650 | 22.610 | 22.830 | 22.280 |
| switch_to_configuration_boot_s | 15.900 | 17.520 | 0.270 | 20.040 | 20.230 | 0.270 |
| noop_switch_to_configuration_boot_s | 3.490 | 4.050 | 0.090 | 4.000 | 4.370 | 0.110 |
| seal_s | 14.124 | 15.122 |  | 18.081 | 17.805 |  |
| sealed_new_paths | 135 | 135 |  | 135 | 135 |  |
| sealed_new_bytes | 2504112956 | 2504112980 |  | 2504112956 | 2504112980 |  |
