# solar-grabber
Small service to pull data from Deye's SUNxxx Microinverters (not from the cloud) and push it into an database. 
Currently only the SUN600 is supported, and only InfluxDB is supported.

The data transferred is the current power generation, the power generated today and the total power generated:
![oeu](res/grafana.png)
