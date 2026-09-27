package dev.teekasfigure.exporter;
import com.google.gson.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
/** Bundled fallbacks work without Fabric API's resource-pack loader. */
public final class Localizations {
    private static final Map<String,String> EN=load("en_us"),RU=load("ru_ru");
    private static Map<String,String> load(String locale) {
        String path="/assets/teekasfigure_mob_exporter/lang/"+locale+".json";
        try(var stream=Localizations.class.getResourceAsStream(path)) {
            if(stream==null)throw new IOException("Missing bundled translations: "+path);
            var root=JsonParser.parseReader(new InputStreamReader(stream,StandardCharsets.UTF_8)).getAsJsonObject();
            Map<String,String> result=new HashMap<>();root.entrySet().forEach(e -> result.put(e.getKey(),e.getValue().getAsString()));return Map.copyOf(result);
        } catch(IOException error){throw new IllegalStateException(error);}
    }
    public static String fallback(String key,String locale){return (locale!=null && locale.startsWith("ru")?RU:EN).getOrDefault(key,EN.getOrDefault(key,"TeekasFigure"));}
}
