package dev.teekasfigure.exporter;
import java.lang.management.ManagementFactory;
import java.nio.file.*;
import java.time.Instant;
import java.util.concurrent.*;
/** One-second heartbeats; a stalled playback writes one diagnostic, never kills threads. */
public final class PlaybackWatchdog {
    private static volatile long clientBeat=System.nanoTime(),serverBeat=clientBeat;
    private static volatile boolean armed,paused;
    private static volatile Path destination;
    private static final ScheduledExecutorService TIMER=Executors.newSingleThreadScheduledExecutor(r -> {var t=new Thread(r,"TeekasFigure stall diagnostic");t.setDaemon(true);return t;});
    static {TIMER.scheduleWithFixedDelay(PlaybackWatchdog::check,1,1,TimeUnit.SECONDS);}
    public static void arm(Path gameDirectory){destination=gameDirectory.toAbsolutePath().resolve("teekasfigure_exports/playback-stall.txt");resume();}
    public static void resume(){clientBeat=serverBeat=System.nanoTime();paused=false;armed=true;}
    public static void disarm(){armed=false;}
    public static void clientPulse(boolean gamePaused){clientBeat=System.nanoTime();paused=gamePaused;}
    public static void serverPulse(){serverBeat=System.nanoTime();}
    private static void check() {
        if(!armed || destination==null || paused)return;
        long now=System.nanoTime();
        if(now-clientBeat<10_000_000_000L && now-serverBeat<10_000_000_000L)return;
        armed=false;
        try {
            var bean=ManagementFactory.getThreadMXBean();var deadlocks=bean.findDeadlockedThreads();
            var text=new StringBuilder("TeekasFigure playback stall "+Instant.now()+"\n");
            text.append("Client heartbeat seconds: ").append((now-clientBeat)/1e9).append("\nServer heartbeat seconds: ").append((now-serverBeat)/1e9).append("\nDeadlocked threads: ").append(java.util.Arrays.toString(deadlocks)).append('\n');
            for(var info:bean.dumpAllThreads(true,true)) {
                text.append('\n').append(info.getThreadName()).append(" [").append(info.getThreadId()).append("] ").append(info.getThreadState()).append(" lock ").append(info.getLockName()).append(" owner ").append(info.getLockOwnerName()).append('\n');
                for(var frame:info.getStackTrace())text.append("    at ").append(frame).append('\n');
            }
            Files.createDirectories(destination.getParent());Files.writeString(destination,text);
            org.slf4j.LoggerFactory.getLogger("TeekasFigure").error("Playback stalled. Thread diagnostic: {}",destination);
        } catch(Exception error){org.slf4j.LoggerFactory.getLogger("TeekasFigure").warn("Could not write playback stall diagnostic",error);}
    }
}
