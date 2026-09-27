package dev.teekasfigure.exporter;

import com.google.gson.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;

/** Validate/pack once, then seek a compact temporary timeline during playback. */
public final class SceneTimeline implements AutoCloseable {
    public enum View { TOP, FRONT }
    public record Brush(String type,double span,double yaw,double depth,double minDepth,double pivotX,double pivotY,double pivotZ,String pose) {}
    public record Shape(long id,int brush,double x,double y,double size,double rotation) {}
    public record Frame(long index,List<Shape> shapes,double[] bottoms,double height) {}
    public final Path folder;
    public final double fps,width,height;
    public final long count;
    public final Brush[] brushes;
    public final View view;
    public final int peakMobs;
    public final double pixelsPerBlock,maxStackHeight;
    private final AtomicBoolean cancelled;
    private BufferedReader reader;
    private long nextIndex;
    private Frame current;
    private Path playbackCache;
    private RandomAccessFile playbackReader;
    private long[] offsets;
    private final char[] textBuffer=new char[8192];
    private int textCursor,textLength;
    private final LayerPacker.Footprint[] footprints;

    public SceneTimeline(Path manifestPath,long budget,AtomicBoolean cancelled,AtomicInteger progress) throws IOException {
        this.cancelled=cancelled;
        folder=manifestPath.toAbsolutePath().getParent();
        JsonObject root=readObject(manifestPath,4*1024*1024);
        if (root.get("schema_version").getAsInt()!=1 || !root.get("complete").getAsBoolean()) throw new IOException("Scene must be complete, schema version 1");
        JsonArray dimensions=root.getAsJsonArray("canvas_px");
        width=positive(dimensions.get(0)); height=positive(dimensions.get(1)); fps=positive(root.get("fps"));
        count=root.get("frame_count").getAsLong(); if(count<=0) throw new IOException("Scene has no frames");
        var runtime=Runtime.getRuntime();
        long availableHeap=runtime.maxMemory()-runtime.totalMemory()+runtime.freeMemory();
        if(count>=Integer.MAX_VALUE || count>availableHeap/64)throw new IOException("Timeline index exceeds safe Java memory budget");
        brushes=new Brush[root.getAsJsonArray("brushes").size()];
        footprints=new LayerPacker.Footprint[brushes.length];
        Arrays.fill(footprints,LayerPacker.Footprint.solid());
        for(JsonElement element:root.getAsJsonArray("brushes")) {
            JsonObject row=element.getAsJsonObject();
            if(!row.has("texture"))continue;
            int index=row.get("index").getAsInt();if(index<0 || index>=brushes.length)throw new IOException("Invalid texture index");
            Path texture=folder.resolve(row.get("texture").getAsString()).normalize();
            if(!texture.startsWith(folder))throw new IOException("Brush texture is outside the scene folder");
            if(cancelled.get())throw new IOException("Cancelled");
            footprints[index]=LayerPacker.Footprint.read(texture);
        }
        JsonArray mappings=JsonParser.parseString(readText(folder.resolve("minecraft_mapping.json"),4*1024*1024)).getAsJsonArray();
        View mappedView=null;
        for(JsonElement element:mappings) {
            JsonObject row=element.getAsJsonObject(); int index=row.get("brush_index").getAsInt();
            if(index<0 || index>=brushes.length || brushes[index]!=null) throw new IOException("Invalid brush index");
            if(row.get("entity_type")==null || row.get("entity_type").isJsonNull()) continue;
            String viewName=row.has("view") && !row.get("view").isJsonNull()?row.get("view").getAsString():"top";
            View rowView=switch(viewName){case "top" -> View.TOP;case "front" -> View.FRONT;default -> throw new IOException("Unsupported mob capture view: "+viewName);};
            if(mappedView!=null && mappedView!=rowView)throw new IOException("A scene cannot mix top-view and front-view mob captures");
            mappedView=rowView;
            String type=row.get("entity_type").getAsString();
            if(!type.matches("[a-z0-9_.-]+:[a-z0-9_./-]+")) throw new IOException("Invalid entity type");
            JsonArray size=row.getAsJsonArray("reference_size_blocks"); double span=positive(size.get(0));
            if(Math.abs(positive(size.get(1))-span)>span*0.001) throw new IOException("Mob captures must be square");
            JsonArray pivot=row.getAsJsonArray("pivot_blocks");
            brushes[index]=new Brush(type,span,value(row.get("yaw_offset_degrees")),positive(row.get("depth_blocks")),
                row.has("min_depth_blocks")?value(row.get("min_depth_blocks")):0,
                value(pivot.get(0)),value(pivot.get(1)),value(pivot.get(2)),row.has("pose")?row.get("pose").getAsString():"");
        }
        view=mappedView==null?View.TOP:mappedView;
        int peak=0; double peakDepthPixels=0,maxScaleNumerator=0;
        offsets=new long[(int)count+1];
        playbackCache=Files.createTempFile("teekasfigure-playback-",".bin");
        try {
          open();
          try(var cache=new DataOutputStream(new BufferedOutputStream(Files.newOutputStream(playbackCache)))) {
            for(long i=0;i<count;i++) {
                Frame frame=readFrame(); if(frame==null) throw new IOException("Truncated timeline");
                peak=Math.max(peak,frame.shapes.size());
                if(peak>budget) throw new IOException("Mobs: "+peak+"; replay budget: "+budget+". Increase the budget to try this scene.");
                for(Shape shape:frame.shapes) {
                    Brush brush=brushes[shape.brush];
                    double numerator=shape.size/brush.span;
                    maxScaleNumerator=Math.max(maxScaleNumerator,numerator);
                }
                peakDepthPixels=Math.max(peakDepthPixels,frame.height());
                long bytes=20L+52L*frame.shapes().size();
                if(Files.getFileStore(playbackCache).getUsableSpace()<bytes+256L*1024*1024)
                    throw new IOException("Not enough temporary disk space for playback cache");
                offsets[(int)i+1]=Math.addExact(offsets[(int)i],bytes);
                cache.writeLong(frame.index());cache.writeInt(frame.shapes().size());cache.writeDouble(frame.height());
                for(int j=0;j<frame.shapes().size();j++) {
                    var shape=frame.shapes().get(j);
                    cache.writeLong(shape.id());cache.writeInt(shape.brush());cache.writeDouble(shape.x());cache.writeDouble(shape.y());
                    cache.writeDouble(shape.size());cache.writeDouble(shape.rotation());cache.writeDouble(frame.bottoms()[j]);
                }
                progress.set((int)Math.min(Integer.MAX_VALUE,i+1));
            }
            if(readFrame()!=null) throw new IOException("Extra frames beyond manifest count");
          } finally {reader.close();reader=null;}
        peakMobs=peak;
        // Keep a stage within a small group of chunks and avoid enormous models.
        // Uniformly scale the whole stage: preserve the picture while keeping
        // giant mob hitboxes/rendering and the camera height manageable.
        pixelsPerBlock=Math.max(16,Math.max(Math.max(width,height)/80,Math.max(maxScaleNumerator/8,peakDepthPixels/128)));
        maxStackHeight=peakDepthPixels/pixelsPerBlock;
        if(!Double.isFinite(pixelsPerBlock) || !Double.isFinite(maxStackHeight) || maxStackHeight>1_000_000) throw new IOException("Scene is too deep for a safe world stage");
        long estimatedBytes=(long)peak*96*1024+64L*1024*1024;
        if(estimatedBytes>(runtime.maxMemory()-runtime.totalMemory()+runtime.freeMemory())*0.65) throw new IOException("Not enough Java heap for "+peak+" world mobs. Reduce scene size or increase Minecraft memory.");
        playbackReader=new RandomAccessFile(playbackCache.toFile(),"r");
        current=readCached(0);
        } catch(IOException | RuntimeException | Error error) {
            if(playbackReader!=null)try{playbackReader.close();}catch(IOException ignored){}
            Files.deleteIfExists(playbackCache);
            throw error;
        }
    }

    public synchronized Frame advanceTo(long index) throws IOException {
        if(cancelled.get())throw new IOException("Cancelled");
        index=Math.max(0,Math.min(count-1,index));
        if(index>current.index())current=readCached(index);
        return current;
    }

    private Frame readCached(long index)throws IOException {
        if(cancelled.get())throw new IOException("Cancelled");
        // Bulk read rather than thousands of unbuffered readDouble syscalls.
        int bytes=Math.toIntExact(offsets[(int)index+1]-offsets[(int)index]);
        byte[] data=new byte[bytes];playbackReader.seek(offsets[(int)index]);playbackReader.readFully(data);
        try(var input=new DataInputStream(new ByteArrayInputStream(data))) {
            long frameIndex=input.readLong();int size=input.readInt();double height=input.readDouble();
            var shapes=new ArrayList<Shape>(size);double[] bottoms=new double[size];
            for(int i=0;i<size;i++) {
                if((i&255)==0 && cancelled.get())throw new IOException("Cancelled");
                shapes.add(new Shape(input.readLong(),input.readInt(),input.readDouble(),input.readDouble(),input.readDouble(),input.readDouble()));
                bottoms[i]=input.readDouble();
            }
            return new Frame(frameIndex,List.copyOf(shapes),bottoms,height);
        }
    }

    private void open() throws IOException { reader=Files.newBufferedReader(folder.resolve("frames.jsonl")); nextIndex=0;textCursor=textLength=0; }
    private Frame readFrame() throws IOException {
        if(cancelled.get()) throw new IOException("Cancelled");
        StringBuilder line=new StringBuilder();
        boolean end=false;
        while(!end) {
            if(cancelled.get())throw new IOException("Cancelled");
            if(textCursor==textLength) {
                textLength=reader.read(textBuffer);textCursor=0;
                if(textLength<0) {
                    if(line.isEmpty())return null;
                    throw new IOException("Incomplete frame line");
                }
            }
            int start=textCursor;
            while(textCursor<textLength && textBuffer[textCursor]!='\n')textCursor++;
            int bytes=textCursor-start;
            if(line.length()+bytes>32*1024*1024)throw new IOException("Frame exceeds safe decoder memory");
            line.append(textBuffer,start,bytes);
            if(textCursor<textLength){textCursor++;end=true;}
        }
        JsonObject root=JsonParser.parseString(line.toString()).getAsJsonObject();
        long index=root.get("frame_index").getAsLong();
        if(index!=nextIndex++) throw new IOException("Non-contiguous frame index");
        if(Math.abs(value(root.get("time_seconds"))-index/fps)>1e-6) throw new IOException("Invalid frame timestamp");
        ArrayList<Shape> result=new ArrayList<>(); HashSet<Long> ids=new HashSet<>();
        for(JsonElement element:root.getAsJsonArray("shapes")) {
            JsonObject p=element.getAsJsonObject(); long id=p.get("id").getAsLong(); int brush=p.get("brush_index").getAsInt();
            if(id<0 || !ids.add(id) || brush<0 || brush>=brushes.length || brushes[brush]==null) throw new IOException("Invalid ID or missing automatic mob mapping");
            if(!p.get("original_colors").getAsBoolean() || Math.abs(value(p.get("opacity"))-1)>1e-4 || Math.abs(value(p.get("hue_turns")))>1e-4 || Math.abs(value(p.get("saturation"))-1)>1e-4 || Math.abs(value(p.get("brightness"))-1)>1e-4)
                throw new IOException("Use real-mob settings: original colors, unchanged HSV, no opacity or fade interpolation");
            JsonArray size=p.getAsJsonArray("size_px"); double sx=positive(size.get(0)),sy=positive(size.get(1));
            if(Math.abs(sx-sy)>sx*0.001) throw new IOException("Mobs require uniform scaling");
            JsonArray pos=p.getAsJsonArray("center_px");
            double x=value(pos.get(0)),y=value(pos.get(1));
            if(x<0 || y<0 || x>width || y>height) throw new IOException("Placement is outside canvas");
            double rotation=value(p.get("rotation_radians"));
            if(view==View.FRONT && Math.abs(rotation)>1e-4)throw new IOException("Front-view mobs require rotation to be disabled");
            result.add(new Shape(id,brush,x,y,sx,rotation));
        }
        var shapes=List.copyOf(result);
        var layout=LayerPacker.pack(shapes,brushes,footprints,width,height,cancelled);
        return new Frame(index,shapes,layout.bottoms(),layout.height());
    }

    public static double value(JsonElement p) throws IOException {
        if(p==null || !p.isJsonPrimitive() || !p.getAsJsonPrimitive().isNumber()) throw new IOException("Expected a number");
        double v=p.getAsDouble(); if(!Double.isFinite(v)) throw new IOException("Non-finite value"); return v;
    }
    public static double positive(JsonElement p) throws IOException { double v=value(p); if(v<=0) throw new IOException("Expected positive value"); return v; }
    private static String readText(Path path,int max) throws IOException {
        if(Files.size(path)>max) throw new IOException("Metadata exceeds safe memory limit");
        return Files.readString(path);
    }
    private static JsonObject readObject(Path path,int max) throws IOException { return JsonParser.parseString(readText(path,max)).getAsJsonObject(); }
    public void cancel(){cancelled.set(true);}
    @Override public synchronized void close() throws IOException {
        cancel();
        try {if(reader!=null)reader.close();}
        finally {try{if(playbackReader!=null)playbackReader.close();}finally{if(playbackCache!=null)Files.deleteIfExists(playbackCache);}}
    }
}
