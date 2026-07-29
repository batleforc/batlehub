# Maven

Proxy a Maven Central-compatible repository — POMs, JARs, source/Javadoc JARs, SHA-1/MD5 checksums, and `maven-metadata.xml` — or host private artifacts. Works with Maven, Gradle, and any tool that speaks the Maven repository protocol.

## At a glance

| | |
|---|---|
| **Config type** | `maven` |
| **Default upstream** | `repo1.maven.org` |
| **Modes** | proxy · local · hybrid |
| **Addressing** | per-package |
| **Private publish** | ✅ `mvn deploy` |

## Proxy setup

Add a mirror to `~/.m2/settings.xml`:

```xml
<settings>
  <mirrors>
    <mirror>
      <id>batlehub</id>
      <mirrorOf>central</mirrorOf>
      <url>https://batlehub.example.com/proxy/<registry>/maven2/</url>
    </mirror>
  </mirrors>
</settings>
```

For Gradle, add the repository in `settings.gradle.kts`:

```kotlin
dependencyResolutionManagement {
    repositories {
        maven { url = uri("https://batlehub.example.com/proxy/<registry>/maven2/") }
    }
}
```

## Publishing (local / hybrid)

The registry must be in `local` or `hybrid` mode. Declare the deploy target in `pom.xml` — the `<id>` must match a `<server>` entry in `settings.xml`:

```xml
<distributionManagement>
  <repository>
    <id>internal-maven</id>
    <url>https://batlehub.example.com/proxy/<registry>/maven2</url>
  </repository>
  <snapshotRepository>
    <id>internal-maven</id>
    <url>https://batlehub.example.com/proxy/<registry>/maven2</url>
  </snapshotRepository>
</distributionManagement>
```

Then deploy:

```bash
mvn deploy
# or override the repository URL without editing pom.xml:
mvn deploy -DaltDeploymentRepository=internal-maven::https://batlehub.example.com/proxy/<registry>/maven2
```

Maven uploads the `.jar`, `-sources.jar`, `.pom`, and checksum files individually; BatleHub records the version when the `.pom` arrives. For Gradle, add a credentialed `maven { … }` repository under `publishing { repositories { … } }` and run `./gradlew publish`.

## Authentication

Credentials live in a `<server>` block in `~/.m2/settings.xml`, keyed by an `<id>` that matches the repository id in `<distributionManagement>`:

```xml
<settings>
  <servers>
    <server>
      <id>internal-maven</id>
      <username>token</username>
      <password>${env.BATLEHUB_TOKEN}</password>
    </server>
  </servers>
</settings>
```

## Notes

Verify a published artifact by fetching its metadata:

```bash
curl -H "Authorization: Bearer $BATLEHUB_TOKEN" \
  "https://batlehub.example.com/proxy/<registry>/maven2/com/example/mylib/maven-metadata.xml"
```

## See also

- [User Guide → Maven](/guide/user#registries)
- [Registries overview](/registries/) · [Caching](/guide/caching) · [Access Control](/guide/access-control)
