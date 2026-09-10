-- Synthetic pre-migration store, including a completed, user-confirmed item.
PRAGMA foreign_keys=ON;
CREATE TABLE metadata(key TEXT PRIMARY KEY,value INTEGER NOT NULL);
CREATE TABLE sources(path TEXT PRIMARY KEY,position INTEGER NOT NULL);
CREATE TABLE episodes(id INTEGER PRIMARY KEY,source TEXT NOT NULL,position INTEGER NOT NULL,speaker TEXT NOT NULL,time INTEGER NOT NULL,text TEXT NOT NULL,fingerprint TEXT NOT NULL,state INTEGER NOT NULL DEFAULT 0,attempts INTEGER NOT NULL DEFAULT 0,UNIQUE(source,position,fingerprint));
CREATE INDEX episode_pending ON episodes(state,id);
CREATE TABLE claims(id INTEGER PRIMARY KEY,scope TEXT NOT NULL,subject TEXT NOT NULL,field TEXT NOT NULL,text TEXT NOT NULL,kind TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'reported',is_open INTEGER NOT NULL,updated INTEGER NOT NULL,origin TEXT NOT NULL);
CREATE INDEX claim_identity ON claims(scope,subject,field);
CREATE TABLE evidence(claim INTEGER NOT NULL REFERENCES claims(id) ON DELETE CASCADE,episode INTEGER NOT NULL REFERENCES episodes(id) ON DELETE CASCADE,PRIMARY KEY(claim,episode));
CREATE TABLE requests(time INTEGER NOT NULL);
CREATE TABLE corrections(id INTEGER PRIMARY KEY,claim INTEGER REFERENCES claims(id) ON DELETE CASCADE,time INTEGER NOT NULL,action TEXT NOT NULL,previous TEXT NOT NULL);
INSERT INTO metadata VALUES('first_enabled',100);
INSERT INTO sources VALUES('session-a',80);
INSERT INTO episodes VALUES(23,'session-a',0,'YOU',100,'Orion needs QA approval.','synthetic-fixture',1,0);
INSERT INTO claims VALUES(41,'orion','deployment','approval','QA approval is pending.','commitment','user_confirmed',0,100,'YOU');
INSERT INTO evidence VALUES(41,23);
INSERT INTO corrections VALUES(1,41,100,'done','QA approval is pending.');
INSERT INTO requests VALUES(100);
PRAGMA user_version=1;
