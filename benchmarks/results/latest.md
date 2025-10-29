# Synthetic performance benchmark summary
- Generated: 2025-10-29T07:50:33.612+00:00
- Seed: 3032024109
- Overall average latency: 225.36 ms
- Worst 95th percentile latency: 457.53 ms
- Overall drop rate: 1.91%
- Pass status: PASS

| Scenario | Stress | Resolution | FPS | Bitrate (Mbps) | Avg (ms) | P95 (ms) | Max (ms) | Drop % | Pass |
|---|---|---|---|---|---|---|---|---|---|
| 1080p_60fps_6mbps | elevated | 1920x1080 | 60 | 6.0 | 187.93 | 257.86 | 369.08 | 1.28 | ✅ |
| 1080p_60fps_6mbps | extreme | 1920x1080 | 60 | 6.0 | 233.22 | 322.96 | 498.44 | 3.15 | ✅ |
| 1080p_60fps_6mbps | nominal | 1920x1080 | 60 | 6.0 | 143.21 | 194.25 | 260.25 | 0.13 | ✅ |
| 1440p_60fps_10mbps | elevated | 2560x1440 | 60 | 10.0 | 219.34 | 305.98 | 444.25 | 1.48 | ✅ |
| 1440p_60fps_10mbps | extreme | 2560x1440 | 60 | 10.0 | 276.32 | 385.69 | 498.15 | 3.57 | ✅ |
| 1440p_60fps_10mbps | nominal | 2560x1440 | 60 | 10.0 | 169.68 | 233.48 | 338.01 | 0.33 | ✅ |
| 2160p_60fps_18mbps | elevated | 3840x2160 | 60 | 18.0 | 279.73 | 398.02 | 497.83 | 2.02 | ✅ |
| 2160p_60fps_18mbps | extreme | 3840x2160 | 60 | 18.0 | 327.12 | 457.53 | 499.68 | 4.87 | ✅ |
| 2160p_60fps_18mbps | nominal | 3840x2160 | 60 | 18.0 | 223.69 | 311.37 | 451.00 | 0.69 | ✅ |

## 1080p_60fps_6mbps [elevated]

**Drop samples**:
- Frame 89: queue_overflow drop at 244.71 ms
- Frame 99: packet_loss drop at 54.50 ms
- Frame 210: queue_overflow drop at 277.06 ms
- Frame 249: queue_overflow drop at 247.31 ms
- Frame 257: packet_loss drop at 51.92 ms
- Frame 404: packet_loss drop at 56.66 ms
- Frame 409: packet_loss drop at 55.57 ms
- Frame 421: queue_overflow drop at 224.04 ms
- Frame 489: queue_overflow drop at 242.64 ms
- Frame 543: queue_overflow drop at 175.16 ms
- Frame 568: packet_loss drop at 26.74 ms
- Frame 638: queue_overflow drop at 141.94 ms

## 1080p_60fps_6mbps [extreme]

**Notes**:
- Drop rate 3.15% exceeded the 3% advisory threshold

**Drop samples**:
- Frame 23: queue_overflow drop at 149.73 ms
- Frame 46: queue_overflow drop at 232.38 ms
- Frame 54: packet_loss drop at 86.16 ms
- Frame 62: queue_overflow drop at 265.03 ms
- Frame 81: packet_loss drop at 83.49 ms
- Frame 119: packet_loss drop at 61.03 ms
- Frame 124: packet_loss drop at 75.01 ms
- Frame 125: packet_loss drop at 49.37 ms
- Frame 135: queue_overflow drop at 116.12 ms
- Frame 156: queue_overflow drop at 255.17 ms
- Frame 175: queue_overflow drop at 357.56 ms
- Frame 196: packet_loss drop at 94.69 ms

## 1080p_60fps_6mbps [nominal]

**Drop samples**:
- Frame 2342: packet_loss drop at 43.81 ms
- Frame 2764: packet_loss drop at 53.31 ms
- Frame 3329: packet_loss drop at 42.98 ms
- Frame 3361: packet_loss drop at 54.49 ms
- Frame 5141: packet_loss drop at 52.34 ms
- Frame 5348: packet_loss drop at 62.15 ms
- Frame 5368: packet_loss drop at 53.38 ms

## 1440p_60fps_10mbps [elevated]

**Drop samples**:
- Frame 103: queue_overflow drop at 199.01 ms
- Frame 177: packet_loss drop at 87.89 ms
- Frame 279: packet_loss drop at 84.30 ms
- Frame 302: packet_loss drop at 98.72 ms
- Frame 354: queue_overflow drop at 206.53 ms
- Frame 373: packet_loss drop at 78.83 ms
- Frame 434: queue_overflow drop at 182.11 ms
- Frame 464: packet_loss drop at 67.51 ms
- Frame 490: queue_overflow drop at 262.87 ms
- Frame 504: queue_overflow drop at 171.61 ms
- Frame 628: queue_overflow drop at 257.74 ms
- Frame 640: packet_loss drop at 74.80 ms

## 1440p_60fps_10mbps [extreme]

**Notes**:
- Drop rate 3.57% exceeded the 3% advisory threshold

**Drop samples**:
- Frame 85: queue_overflow drop at 396.77 ms
- Frame 114: queue_overflow drop at 205.12 ms
- Frame 150: packet_loss drop at 92.14 ms
- Frame 157: packet_loss drop at 99.94 ms
- Frame 162: packet_loss drop at 102.17 ms
- Frame 232: queue_overflow drop at 369.31 ms
- Frame 244: queue_overflow drop at 301.90 ms
- Frame 248: queue_overflow drop at 256.14 ms
- Frame 268: packet_loss drop at 118.41 ms
- Frame 283: packet_loss drop at 66.91 ms
- Frame 353: packet_loss drop at 54.38 ms
- Frame 356: packet_loss drop at 119.05 ms

## 1440p_60fps_10mbps [nominal]

**Drop samples**:
- Frame 130: queue_overflow drop at 201.80 ms
- Frame 1155: queue_overflow drop at 112.29 ms
- Frame 1719: packet_loss drop at 57.55 ms
- Frame 1733: queue_overflow drop at 119.70 ms
- Frame 1948: packet_loss drop at 62.84 ms
- Frame 2494: packet_loss drop at 58.51 ms
- Frame 2651: packet_loss drop at 58.06 ms
- Frame 2837: queue_overflow drop at 167.34 ms
- Frame 2872: queue_overflow drop at 95.00 ms
- Frame 3217: packet_loss drop at 64.20 ms
- Frame 3233: queue_overflow drop at 138.18 ms
- Frame 3469: queue_overflow drop at 177.21 ms

## 2160p_60fps_18mbps [elevated]

**Drop samples**:
- Frame 40: queue_overflow drop at 227.42 ms
- Frame 44: latency_budget drop at 531.25 ms
- Frame 94: packet_loss drop at 74.59 ms
- Frame 109: packet_loss drop at 113.07 ms
- Frame 129: latency_budget drop at 543.22 ms
- Frame 188: packet_loss drop at 110.94 ms
- Frame 237: queue_overflow drop at 368.19 ms
- Frame 307: packet_loss drop at 138.46 ms
- Frame 389: packet_loss drop at 116.88 ms
- Frame 443: queue_overflow drop at 266.27 ms
- Frame 471: queue_overflow drop at 257.15 ms
- Frame 534: queue_overflow drop at 133.06 ms

## 2160p_60fps_18mbps [extreme]

**Notes**:
- Drop rate 4.87% exceeded the 3% advisory threshold
- P95 latency 457.53 ms is within 10% of the budget (500 ms)

**Drop samples**:
- Frame 11: queue_overflow drop at 242.86 ms
- Frame 39: queue_overflow drop at 331.02 ms
- Frame 52: latency_budget drop at 511.05 ms
- Frame 55: queue_overflow drop at 238.58 ms
- Frame 127: latency_budget drop at 509.28 ms
- Frame 171: packet_loss drop at 71.01 ms
- Frame 188: packet_loss drop at 172.71 ms
- Frame 224: latency_budget drop at 503.76 ms
- Frame 255: latency_budget drop at 551.52 ms
- Frame 300: queue_overflow drop at 331.68 ms
- Frame 301: queue_overflow drop at 419.37 ms
- Frame 326: packet_loss drop at 148.29 ms

## 2160p_60fps_18mbps [nominal]

**Drop samples**:
- Frame 132: packet_loss drop at 83.44 ms
- Frame 318: queue_overflow drop at 272.48 ms
- Frame 591: packet_loss drop at 83.50 ms
- Frame 645: packet_loss drop at 90.33 ms
- Frame 720: queue_overflow drop at 256.66 ms
- Frame 737: packet_loss drop at 100.90 ms
- Frame 998: packet_loss drop at 82.22 ms
- Frame 1157: packet_loss drop at 81.88 ms
- Frame 1181: packet_loss drop at 106.70 ms
- Frame 1362: packet_loss drop at 65.85 ms
- Frame 1524: packet_loss drop at 79.92 ms
- Frame 1531: queue_overflow drop at 302.69 ms
