package dev.teekasfigure.exporter;
import net.minecraft.entity.Entity;

/** Data-tracked names synchronize scale/ownership without custom network packets. */
public final class Participant {
    public static final String PREFIX="[TeekasFigure:";
    public record Info(boolean owned,float scale,String pose) {}
    private static final Info UNOWNED=new Info(false,1,"");
    public static Info metadata(Entity entity) {
        var name=entity.getCustomName();
        return entity instanceof ParticipantCache cache?cache.teekasfigure$metadata(name):parse(name);
    }
    public static Info parse(net.minecraft.text.Text name) {
        if(name==null)return UNOWNED;
        String text=name.getString();if(!text.startsWith(PREFIX))return UNOWNED;
        try {
            int last=text.lastIndexOf(':'),before=text.lastIndexOf(':',last-1);
            float scale=Float.parseFloat(text.substring(last+1,text.length()-1));
            String pose=before>=0?text.substring(before+1,last):"";
            return new Info(true,Float.isFinite(scale) && scale>0 && scale<=64?scale:1,pose);
        }catch(RuntimeException error){return new Info(true,1,"");}
    }
    public static boolean owned(Entity entity) {
        return metadata(entity).owned();
    }
    public static float scale(Entity entity) {
        return metadata(entity).scale();
    }
}
