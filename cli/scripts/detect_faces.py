#!/usr/bin/env python3
"""Detect faces in an image. Output JSON to stdout."""
import json, sys

if len(sys.argv) < 2:
    print("Usage: detect_faces.py <image_path>", file=sys.stderr)
    sys.exit(1)

try:
    import cv2
    cascade = cv2.CascadeClassifier(cv2.data.haarcascades + 'haarcascade_frontalface_default.xml')
    img = cv2.imread(sys.argv[1])
    gray = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)
    h, w = gray.shape
    faces = cascade.detectMultiScale(gray, 1.1, 5, minSize=(int(w*0.03), int(h*0.03)))
    result = [{"x": int(x), "y": int(y), "w": int(fw), "h": int(fh),
               "cx": round((x+fw/2)/w, 3), "cy": round((y+fh/2)/h, 3),
               "coverage": round((fw*fh)/(w*h), 4)} for (x,y,fw,fh) in faces]
    json.dump(result, sys.stdout)
except ImportError:
    json.dump([], sys.stdout)  # graceful degrade if no opencv
except Exception as e:
    print(f"detect_faces: {e}", file=sys.stderr)
    json.dump([], sys.stdout)
