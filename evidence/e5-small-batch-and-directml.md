# E5 Small: batch size и DirectML capability

Дата: 2026-08-16. Устройство: Intel Core i7-14700KF, NVIDIA GeForce GTX 1050 Ti 4 GiB. Модель: `intfloat/multilingual-e5-small@614241f622f53c4eeff9890bdc4f31cfecc418b3`.

Команда: `cargo run --release --example batch_benchmark -- D:\Igor\Programming\FastSearch`.

Метод: 128 реальных text/source файлов до 4000 символов, один загруженный CPU runtime, отдельный warm-up, медиана трёх полных проходов на batch `1/2/4/8/16/32/64`. Memory — process working set после соответствующего прохода, не аппаратный peak counter.

| Batch | Время, ms | Док/с | Working set |
|---:|---:|---:|---:|
| 1 | 3138 | 40.78 | 0.95 GiB |
| 2 | 3291 | 38.89 | 1.03 GiB |
| 4 | 3875 | 33.03 | 1.08 GiB |
| 8 | 4155 | 30.80 | 1.31 GiB |
| 16 | 4201 | 30.47 | 1.74 GiB |
| 32 | 4323 | 29.60 | 2.60 GiB |
| 64 | 4417 | 28.98 | 5.08 GiB |

Решение: сохранить batch `1` для heterogeneous corpus. Следующая независимая гипотеза — length bucketing с восстановлением исходного порядка; увеличение batch без bucketing отклонено.

DirectML probe на GTX 1050 Ti завершён успешно: runtime вернул finite embedding размерности 384. Machine-local evidence сохранён в `runtime-capabilities.toml` как `cpu = ready`, `gpu = ready`, `gpu_backend = DirectML`. Эта проба доказывает возможность запуска E5 Small, но ещё не переключает production search с CPU на GPU.

## CPU против GPU

DirectML измерен на тех же 128 текстах и тем же методом. Process working set не включает VRAM и поэтому не используется как оценка видеопамяти. Фактическая GPU utilization/VRAM наблюдалась через `nvidia-smi` во время прогона.

| GPU batch | Время, ms | Док/с | Process working set |
|---:|---:|---:|---:|
| 1 | 6923 | 18.49 | 0.51 GiB |
| 2 | 6323 | 20.24 | 0.52 GiB |
| 4 | 6114 | 20.93 | 0.53 GiB |
| 8 | 5743 | 22.29 | 0.54 GiB |
| 16 | 5571 | 22.97 | 0.55 GiB |
| 32 | 5523 | 23.17 | 0.54 GiB |
| 64 | 6056 | 21.14 | 1.29 GiB |

Лучший CPU: batch `1`, `40.78 docs/s`. Лучший GPU: batch `32`, `23.17 docs/s`. CPU быстрее в `1.76x`. На GPU batch `64` наблюдалось около `3694 MiB / 4096 MiB` VRAM и снижение throughput, поэтому он не является безопасным default для GTX 1050 Ti.

Решение для текущего hardware profile: production embedding остаётся на CPU с batch `1`. DirectML сохраняется как доказанная capability и экспериментальный backend, но автоматический выбор GPU только по наличию `✓` запрещён: device policy должна учитывать benchmark конкретной машины.
