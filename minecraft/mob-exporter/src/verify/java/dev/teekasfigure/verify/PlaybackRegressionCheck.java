package dev.teekasfigure.verify;
import dev.teekasfigure.exporter.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.*;
public final class PlaybackRegressionCheck {
    private static int passed;
    private static void check(boolean value,String description){if(!value)throw new AssertionError(description);passed++;System.out.println("PASS "+description);}
    private static SceneTimeline.Shape q(long id,int brush,double x,double y,double size,double rotation){return new SceneTimeline.Shape(id,brush,x,y,size,rotation);}
    public static void main(String[] args)throws Exception {
        var brush=new SceneTimeline.Brush("minecraft:cat",1,180,1,0,0,0,0,"");
        var brushes=new SceneTimeline.Brush[]{brush,brush};var solid=LayerPacker.Footprint.solid();
        var masks=new LayerPacker.Footprint[]{solid,solid};var token=new AtomicBoolean();
        var layout=LayerPacker.pack(List.of(q(1,0,20,50,10,0),q(2,0,80,50,10,0)),brushes,masks,100,100,token);
        check(layout.bottoms()[0]==0 && layout.bottoms()[1]==0,"disjoint silhouettes share the bottom layer");
        layout=LayerPacker.pack(List.of(q(1,0,20,50,10,0),q(2,0,20,50,10,0)),brushes,masks,100,100,token);
        check(layout.bottoms()[1]>=12,"later overlapping shape remains above the earlier shape");
        layout=LayerPacker.pack(List.of(q(1,0,20,50,10,0),q(2,0,80,50,20,0),q(3,0,20,50,10,0)),brushes,masks,100,100,token);
        check(layout.bottoms()[2]==12,"distant tall shape does not add an air gap to a local stack");
        boolean[] left=new boolean[64*64],right=new boolean[64*64];
        for(int y=0;y<64;y++)for(int x=0;x<64;x++){left[y*64+x]=x<16;right[y*64+x]=x>=48;}
        masks=new LayerPacker.Footprint[]{LayerPacker.Footprint.fromCells(left),LayerPacker.Footprint.fromCells(right)};
        layout=LayerPacker.pack(List.of(q(1,0,50,50,80,0),q(2,1,50,50,80,0)),brushes,masks,100,100,token);
        check(layout.bottoms()[1]==0,"transparent parts of overlapping PNG squares do not force stacking");
        boolean[] fin=new boolean[4096];fin[32*64+32]=true;
        masks=new LayerPacker.Footprint[]{LayerPacker.Footprint.fromCells(fin),solid};
        layout=LayerPacker.pack(List.of(q(1,0,50,50,20,.7),q(2,0,50,50,20,.7)),brushes,masks,100,100,token);
        check(layout.bottoms()[1]>0,"a rotated one-cell fin is never lost by center-only sampling");
        token.set(true);boolean cancelled=false;
        try{LayerPacker.pack(List.of(q(1,0,50,50,20,0)),brushes,masks,100,100,token);}catch(java.io.IOException error){cancelled=true;}
        check(cancelled,"layout work can be cancelled without touching the world");
        String path=System.getProperty("scenePath");
        if(path!=null) {
            long start=System.nanoTime();var progress=new AtomicInteger();
            try(var scene=new SceneTimeline(Path.of(path),256,new AtomicBoolean(),progress)) {
                check(scene.count==125 && scene.peakMobs==256 && progress.get()==125,"user's complete 125-frame, 256-mob scene decodes successfully");
                check(scene.maxStackHeight<=128.000001,"user scene stage height stays within 128 blocks");
                double largest=0;
                for(long i=0;i<scene.count;i++) {
                    var frame=scene.advanceTo(i);
                    checkFrame(frame,scene);
                    for(var shape:frame.shapes())largest=Math.max(largest,shape.size()/scene.pixelsPerBlock/scene.brushes[shape.brush()].span());
                }
                check(largest<=8.000001,"uniform stage scale keeps every giant mob at scale 8 or below");
                double seconds=(System.nanoTime()-start)/1e9;
                check(seconds<30,"full validation and replay of the reported scene completes within 30 seconds without Minecraft");
                System.out.printf(Locale.ROOT,"Reported scene: %.3fs, %.3f pixels/block, %.3f blocks packed height, largest mob scale %.3f%n",seconds,scene.pixelsPerBlock,scene.maxStackHeight,largest);
            }
        }
        System.out.println("Playback regression checks passed: "+passed);
    }
    private static void checkFrame(SceneTimeline.Frame frame,SceneTimeline scene) {
        if(frame.bottoms().length!=frame.shapes().size() || frame.height()/scene.pixelsPerBlock>scene.maxStackHeight+.001)throw new AssertionError("Invalid packed frame bounds");
        for(double bottom:frame.bottoms())if(!Double.isFinite(bottom) || bottom<0)throw new AssertionError("Invalid layer height");
    }
}
